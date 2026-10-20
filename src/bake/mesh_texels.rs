//! Which texels a mesh's bake traces, and from where. Henry's bake traces the texels whose
//! centers lie inside a triangle. Two more rules make thin and intersecting geometry bake right:
//!
//! - **Touched texels.** A texel a triangle touches whose center no triangle holds is traced
//!   from the triangle's nearest point ([`touched_texels`]), so thin strips get their own light.
//! - **Buried texels.** A texel inside another mesh, such as a pipe running into a beam, would
//!   trace its shadows from in there and bake black, and the visible surface next to it reads it.
//!   It looks out mostly at back faces, so the bake skips it and the seam padding fills it from
//!   its visible neighbours.

use bevy::math::Vec3;
use rayon::prelude::*;

use super::bits::{AtomicBitGrid, BitGrid};
use super::mesh_bake::{RAY_OFFSET, TriangleSetup};
use super::mesh_builder::PreparedMesh;
use super::occluder::{Occluders, RayCaster};
use super::random;
use super::texels::{SurfaceTexel, touched_texels, triangle_texels};

/// Rays per texel over the hemisphere above the face.
pub const BURIAL_RAYS: u32 = 8;
/// A texel is buried when at least this many of its rays hit a back face first.
pub const BURIED_AT: u32 = 5;
/// Keeps the burial streams apart from the shadow streams of the same texel.
const STREAM: u64 = 0x6275_7269_6564;

/// One mesh's texel bookkeeping.
pub struct MeshTexels {
    /// Texels whose center lies inside some triangle, by texel.
    owned: BitGrid,
    /// Per triangle, which of its traced texels are buried, in [`traced`] order. Per triangle,
    /// not per texel: where UV1 islands overlap (a cylinder's caps over its side), a buried
    /// cap texel mustn't hide the side's.
    buried: Vec<Vec<bool>>,
}

impl MeshTexels {
    /// Finds the owned texels, then tests every texel the bake would trace for burial. Burial
    /// rays ignore alpha: a cutout surface still has sides.
    pub fn new(
        occluders: &Occluders,
        seed: u64,
        prepared: &PreparedMesh,
        setups: &[TriangleSetup],
    ) -> Self {
        let size = prepared.resolution;
        let owned = AtomicBitGrid::new(size, size);
        setups.par_iter().enumerate().for_each(|(triangle, setup)| {
            if setup.face_normal != Vec3::ZERO {
                for texel in centered(prepared, triangle, setup.face_normal) {
                    owned.set(texel.x, texel.y);
                }
            }
        });
        let owned = owned.into_grid();
        let buried = setups
            .par_iter()
            .enumerate()
            .map(|(triangle, setup)| {
                let normal = setup.face_normal;
                if normal == Vec3::ZERO {
                    return Vec::new();
                }
                let texels = traced(prepared, triangle, normal, &owned);
                texels
                    .iter()
                    .map(|texel| {
                        let origin = texel.position + normal * RAY_OFFSET;
                        let mut rng = random::stream(&[
                            seed,
                            prepared.mesh.id.0,
                            STREAM,
                            triangle as u64,
                            u64::from(texel.x),
                            u64::from(texel.y),
                        ]);
                        let back_face = |direction: Vec3| {
                            occluders
                                .closest_hit(origin, direction, f32::INFINITY, false)
                                .is_some_and(|hit| occluders.face_normal(&hit).dot(direction) > 0.0)
                        };
                        is_buried(normal, rng.next_f32(), back_face)
                    })
                    .collect()
            })
            .collect();
        Self { owned, buried }
    }

    /// The texels of `triangle` the bake traces: its centered and touched texels, unburied.
    pub fn of(
        &self,
        prepared: &PreparedMesh,
        triangle: usize,
        face_normal: Vec3,
    ) -> Vec<SurfaceTexel> {
        let buried = &self.buried[triangle];
        traced(prepared, triangle, face_normal, &self.owned)
            .into_iter()
            .zip(buried.iter().chain(std::iter::repeat(&false)))
            .filter(|(_, buried)| !**buried)
            .map(|(texel, _)| texel)
            .collect()
    }
}

/// Whether the texel facing `normal` is buried. `back_face(direction)` traces one ray from just
/// above the texel and says whether the nearest surface it hits faces away from it. Stops as soon
/// as the answer is known; `turn` (0 to 1) rotates the ray pattern.
pub fn is_buried(normal: Vec3, turn: f32, mut back_face: impl FnMut(Vec3) -> bool) -> bool {
    let (tangent, bitangent) = normal.any_orthonormal_pair();
    let (mut back, mut front) = (0, 0);
    for i in 0..BURIAL_RAYS {
        // Stratified: one ray per azimuth sector, elevations spread over the sectors.
        let ring = ((i * 5) % BURIAL_RAYS) as f32 + 0.5;
        let sin_theta = (ring / BURIAL_RAYS as f32).sqrt();
        let cos_theta = (1.0 - sin_theta * sin_theta).sqrt();
        let phi = (i as f32 + turn) * std::f32::consts::TAU / BURIAL_RAYS as f32;
        let direction =
            (tangent * phi.cos() + bitangent * phi.sin()) * sin_theta + normal * cos_theta;
        if back_face(direction) {
            back += 1;
        } else {
            front += 1;
        }
        if back >= BURIED_AT {
            return true;
        }
        if front > BURIAL_RAYS - BURIED_AT {
            return false;
        }
    }
    false
}

fn centered(prepared: &PreparedMesh, triangle: usize, face_normal: Vec3) -> Vec<SurfaceTexel> {
    triangle_texels(
        prepared.vertices(triangle),
        prepared.normals(triangle),
        prepared.uvs(triangle),
        face_normal,
        prepared.resolution,
    )
}

fn traced(
    prepared: &PreparedMesh,
    triangle: usize,
    face_normal: Vec3,
    owned: &BitGrid,
) -> Vec<SurfaceTexel> {
    let mut texels = centered(prepared, triangle, face_normal);
    texels.extend(touched_texels(
        prepared.vertices(triangle),
        prepared.normals(triangle),
        prepared.uvs(triangle),
        face_normal,
        prepared.resolution,
        |x, y| owned.get(x, y),
    ));
    texels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_cover_the_hemisphere_above_the_face() {
        let mut all = Vec::new();
        is_buried(Vec3::Y, 0.3, |d| {
            all.push(d);
            true
        });
        assert_eq!(all.len(), BURIED_AT as usize, "stops once buried");
        let mut open = Vec::new();
        is_buried(Vec3::Y, 0.3, |d| {
            open.push(d);
            false
        });
        assert_eq!(open.len(), (BURIAL_RAYS - BURIED_AT + 1) as usize);
        for d in all.iter().chain(&open) {
            assert!((d.length() - 1.0).abs() < 1e-5 && d.y > 0.0, "{d}");
        }
    }

    #[test]
    fn a_majority_of_back_faces_buries_the_texel() {
        let mut count = 0;
        assert!(is_buried(Vec3::Z, 0.0, |_| {
            count += 1;
            count != 2
        }));
        let mut count = 0;
        assert!(!is_buried(Vec3::Z, 0.0, |_| {
            count += 1;
            count % 2 == 0
        }));
    }
}
