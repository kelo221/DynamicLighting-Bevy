//! One mesh's bake (Henry's `Raytrace`): associate lights with triangles, trace shadow rays per
//! texel and light (one to a point light, several over an area light or a sun's disk), then hand
//! over to bounce, optimization, seams and serialization.

use bevy::math::Vec3;
use rayon::prelude::*;

use super::bits::{AtomicBitGrid, BitGrid, TexelRect};
use super::bounce::{self, PhotonCube};
use super::format::BakedMesh;
use super::geometry::{Aabb, approx_eq, sphere_intersects_triangle};
use super::input::{BakeError, BakeLight, BakeOptions};
use super::light_bvh::LightBvh;
use super::mesh_builder::{PreparedMesh, crop_rect};
use super::mirror::MAX_TRIANGLE_LIGHTS;
use super::occluder::{Occluders, RayCaster};
use super::progress::BakeProgress;
use super::shadow_grid::{AtomicShadowGrid, ShadowRegion};
use super::shadows;
use super::texel_shadow::texel_shadow;
use super::texels::triangle_texels;
use super::triangle_words::{TriangleRecord, serialize_triangles};

/// Henry's tolerance for surfaces facing slightly away from a light.
pub const NDOTL_TOLERANCE: f32 = -0.1;
/// Shadow rays start this far above the surface, along the face normal.
pub const RAY_OFFSET: f32 = 0.001;

/// Scene-wide state shared by every mesh's bake.
pub struct Scene<'a> {
    pub options: &'a BakeOptions,
    /// Baked lights in BVH order, then the suns in [`crate::shared::BakeId`] order.
    pub lights: Vec<&'a BakeLight>,
    /// The lights in the BVH: all but the trailing suns.
    pub local_lights: usize,
    pub light_bvh: LightBvh,
    pub occluders: Occluders<'a>,
    /// Any light bakes bounce: triangle light entries use the 3-word layout.
    pub bounce_in_scene: bool,
    /// Photon cubes for bounce lights, by BVH slot.
    pub photons: Vec<Option<PhotonCube>>,
    pub progress: &'a BakeProgress,
}

/// A triangle after light association.
pub struct TriangleSetup {
    /// BVH slots of the lights that may reach the triangle, ascending.
    pub lights: Vec<u32>,
    /// Unit face normal, zero for degenerate triangles.
    pub face_normal: Vec3,
    /// Texels whose bits are stored for the triangle (before padding).
    pub crop: TexelRect,
}

/// Per light, the shadow map region its bits may occupy in this mesh.
pub struct LightRegions {
    /// `slot[light]` indexes `rects`, or is `u32::MAX` when no triangle has the light.
    pub slot: Vec<u32>,
    /// `(light, rect)`, ascending by light.
    pub rects: Vec<(u32, TexelRect)>,
}

/// One mesh's results.
pub struct MeshOutput {
    pub mesh: BakedMesh,
    pub texels: u64,
    pub rays: u64,
    pub bounce_rays: u64,
    pub lights_removed: u64,
    pub crowded: Crowding,
}

pub fn bake_mesh(scene: &Scene, prepared: &PreparedMesh) -> Result<MeshOutput, BakeError> {
    let setups: Vec<TriangleSetup> = (0..prepared.triangle_count())
        .into_par_iter()
        .map(|triangle| associate(scene, prepared, triangle))
        .collect();
    let regions = light_regions(&setups, scene.lights.len());

    let traced = trace(scene, prepared, &setups, &regions);
    scene.progress.check()?;

    let (mut bounce, bounce_rays) = bounce::bake_mesh_bounce(scene, prepared, &setups);
    scene.progress.check()?;

    let mut entries = shadows::LightEntries::new(setups, &mut bounce);
    let lights_removed = entries.optimize(prepared, &regions, &traced.bits);
    let mut bits = traced.bits;
    let used = entries.used_lights(scene.lights.len());
    bits.par_iter_mut()
        .zip(&regions.rects)
        .filter(|(_, (light, _))| used[*light as usize])
        .for_each(|(region, _)| super::seams::pad_seams(region, &traced.visited));

    let records = entries.into_records(scene, prepared, &regions, &bits);
    let crowded = Crowding::of(&records);
    let triangles = serialize_triangles(&records, scene.bounce_in_scene);
    Ok(MeshOutput {
        mesh: BakedMesh {
            id: prepared.mesh.id,
            content_hash: prepared.mesh.content_hash,
            resolution: prepared.resolution,
            triangles,
            occludes: prepared.mesh.occludes,
        },
        texels: traced.texels,
        rays: traced.rays,
        bounce_rays,
        lights_removed,
        crowded,
    })
}

/// Triangles with more lights than the shaders read ([`MAX_TRIANGLE_LIGHTS`]); the rest of
/// their lights go missing at runtime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Crowding {
    pub triangles: u64,
    /// The most lights any triangle has.
    pub most_lights: usize,
}

impl Crowding {
    fn of(records: &[TriangleRecord]) -> Self {
        let limit = MAX_TRIANGLE_LIGHTS as usize;
        records
            .iter()
            .map(|record| record.lights.len())
            .filter(|&lights| lights > limit)
            .fold(Self::default(), |crowding, lights| Self {
                triangles: crowding.triangles + 1,
                most_lights: crowding.most_lights.max(lights),
            })
    }
}

/// Henry's light association: lights whose sphere touches the triangle, skipping lights the
/// (flat-shaded) triangle faces away from, unless the light bakes bounce. An area light's sphere
/// holds its shape and everything it reaches past it; suns reach every triangle.
fn associate(scene: &Scene, prepared: &PreparedMesh, triangle: usize) -> TriangleSetup {
    let [a, b, c] = prepared.vertices(triangle);
    let [n1, n2, n3] = prepared.normals(triangle);
    let face_normal = (b - a).cross(c - a).normalize_or_zero();
    let valid = face_normal != Vec3::ZERO;
    let flat = approx_eq(n1, n2, 1e-5) && approx_eq(n2, n3, 1e-5);

    let mut candidates = Vec::new();
    scene
        .light_bvh
        .query(&Aabb::from_points(&[a, b, c]), &mut candidates);
    candidates.extend(scene.local_lights as u32..scene.lights.len() as u32);
    let lights = candidates
        .into_iter()
        .filter(|&slot| {
            let light = scene.lights[slot as usize];
            if valid
                && flat
                && !light.bakes_bounce(scene.options)
                && !light.may_face([a, b, c], face_normal)
            {
                return false;
            }
            light.shape.is_distant()
                || sphere_intersects_triangle(light.position, light.radius, a, b, c)
        })
        .collect();
    TriangleSetup {
        lights,
        face_normal,
        crop: crop_rect(prepared.uvs(triangle), prepared.resolution),
    }
}

fn light_regions(setups: &[TriangleSetup], light_count: usize) -> LightRegions {
    let mut rects: Vec<Option<TexelRect>> = vec![None; light_count];
    for setup in setups {
        for &light in &setup.lights {
            let rect = &mut rects[light as usize];
            *rect = Some(rect.map_or(setup.crop, |r| r.union(setup.crop)));
        }
    }
    let mut slot = vec![u32::MAX; light_count];
    let mut list = Vec::new();
    for (light, rect) in rects.into_iter().enumerate() {
        if let Some(rect) = rect {
            slot[light] = list.len() as u32;
            list.push((light as u32, rect));
        }
    }
    LightRegions { slot, rects: list }
}

struct Traced {
    visited: BitGrid,
    bits: Vec<ShadowRegion>,
    texels: u64,
    rays: u64,
}

/// Henry's `RaycastTriangle`: per texel inside the triangle and per associated light, range and
/// facing early-outs, then shadow rays from just above the surface; a miss lights the texel.
fn trace(
    scene: &Scene,
    prepared: &PreparedMesh,
    setups: &[TriangleSetup],
    regions: &LightRegions,
) -> Traced {
    let size = prepared.resolution;
    let visited = AtomicBitGrid::new(size, size);
    let grids: Vec<AtomicShadowGrid> = regions
        .rects
        .iter()
        .map(|&(light, rect)| {
            let bits = scene.lights[light as usize].shadow_bits_per_texel();
            AtomicShadowGrid::new(rect.width(), rect.height(), bits)
        })
        .collect();

    let (texels, rays) = setups
        .par_iter()
        .enumerate()
        .map(|(triangle, setup)| {
            if setup.face_normal == Vec3::ZERO || scene.progress.is_cancelled() {
                return (0, 0);
            }
            let texels = triangle_texels(
                prepared.vertices(triangle),
                prepared.normals(triangle),
                prepared.uvs(triangle),
                setup.face_normal,
                size,
            );
            let offset = setup.face_normal * RAY_OFFSET;
            let mut rays = 0u64;
            for &light_slot in &setup.lights {
                let light = scene.lights[light_slot as usize];
                let index = regions.slot[light_slot as usize] as usize;
                let (grid, rect) = (&grids[index], regions.rects[index].1);
                for texel in &texels {
                    let origin = texel.position + offset;
                    let occluded = |direction, distance| {
                        let caster = &scene.occluders;
                        caster.occluded(origin, direction, distance, light.alpha_tested())
                    };
                    let traced = texel_shadow(scene, prepared, light, texel, occluded);
                    let Some((lit, traced_rays)) = traced else {
                        continue;
                    };
                    rays += u64::from(traced_rays);
                    visited.set(texel.x, texel.y);
                    grid.raise(texel.x - rect.min_x, texel.y - rect.min_y, lit);
                }
            }
            (texels.len() as u64, rays)
        })
        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));

    Traced {
        visited: visited.into_grid(),
        bits: grids
            .into_iter()
            .zip(&regions.rects)
            .map(|(grid, (_, rect))| ShadowRegion {
                rect: *rect,
                grid: grid.into_grid(),
            })
            .collect(),
        texels,
        rays,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::triangle_words::TriangleLight;

    fn record(lights: u32) -> TriangleRecord {
        TriangleRecord {
            crop: TexelRect {
                min_x: 0,
                min_y: 0,
                max_x: 0,
                max_y: 0,
            },
            lights: (0..lights)
                .map(|light| TriangleLight {
                    light,
                    shadow: None,
                    bounce: None,
                })
                .collect(),
        }
    }

    #[test]
    fn triangles_past_the_shader_light_limit_are_counted() {
        let limit = MAX_TRIANGLE_LIGHTS;
        let fits = [record(3), record(limit)];
        assert_eq!(Crowding::of(&fits), Crowding::default());
        let crowded = [record(limit + 1), record(2), record(limit + 9)];
        let expected = Crowding {
            triangles: 2,
            most_lights: limit as usize + 9,
        };
        assert_eq!(Crowding::of(&crowded), expected);
    }
}
