//! One mesh's bake (Henry's `Raytrace`): associate lights with triangles, trace a shadow ray per
//! texel and light, then hand over to bounce, optimization, seams and serialization.

use bevy::math::Vec3;
use rayon::prelude::*;

use super::bits::{AtomicBitGrid, BitGrid, RegionBits, TexelRect};
use super::bounce::{self, PhotonCube};
use super::format::BakedMesh;
use super::geometry::{Aabb, approx_eq, sphere_intersects_triangle};
use super::input::{BakeError, BakeLight, BakeOptions};
use super::light_bvh::LightBvh;
use super::mesh_builder::{PreparedMesh, crop_rect};
use super::occluder::{Occluders, RayCaster};
use super::progress::BakeProgress;
use super::shadows;
use super::texels::triangle_texels;
use super::triangle_words::serialize_triangles;

/// Henry's tolerance for surfaces facing slightly away from a light.
pub const NDOTL_TOLERANCE: f32 = -0.1;
/// Shadow rays start this far above the surface, along the face normal.
pub const RAY_OFFSET: f32 = 0.001;

/// Scene-wide state shared by every mesh's bake.
pub struct Scene<'a> {
    pub options: &'a BakeOptions,
    /// Baked lights in BVH order.
    pub lights: Vec<&'a BakeLight>,
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
    let triangles = serialize_triangles(&records, scene.bounce_in_scene);
    Ok(MeshOutput {
        mesh: BakedMesh {
            id: prepared.mesh.id,
            content_hash: prepared.mesh.content_hash,
            resolution: prepared.resolution,
            triangles,
        },
        texels: traced.texels,
        rays: traced.rays,
        bounce_rays,
        lights_removed,
    })
}

/// Henry's light association: lights whose sphere touches the triangle, skipping lights the
/// (flat-shaded) triangle faces away from, unless the light bakes bounce.
fn associate(scene: &Scene, prepared: &PreparedMesh, triangle: usize) -> TriangleSetup {
    let [a, b, c] = prepared.vertices(triangle);
    let [n1, n2, n3] = prepared.normals(triangle);
    let face_normal = (b - a).cross(c - a).normalize_or_zero();
    let valid = face_normal != Vec3::ZERO;
    let flat = approx_eq(n1, n2, 1e-5) && approx_eq(n2, n3, 1e-5);
    let center = (a + b + c) / 3.0;

    let mut candidates = Vec::new();
    scene
        .light_bvh
        .query(&Aabb::from_points(&[a, b, c]), &mut candidates);
    let lights = candidates
        .into_iter()
        .filter(|&slot| {
            let light = scene.lights[slot as usize];
            if valid && flat && !light.bakes_bounce(scene.options) {
                let direction = (light.position - center).normalize_or_zero();
                if face_normal.dot(direction) <= NDOTL_TOLERANCE {
                    return false;
                }
            }
            sphere_intersects_triangle(light.position, light.radius, a, b, c)
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
    bits: Vec<RegionBits>,
    texels: u64,
    rays: u64,
}

/// Henry's `RaycastTriangle`: per texel inside the triangle and per associated light, range and
/// facing early-outs, then a shadow ray from just above the surface; a miss lights the texel.
fn trace(
    scene: &Scene,
    prepared: &PreparedMesh,
    setups: &[TriangleSetup],
    regions: &LightRegions,
) -> Traced {
    let size = prepared.resolution;
    let visited = AtomicBitGrid::new(size, size);
    let grids: Vec<AtomicBitGrid> = regions
        .rects
        .iter()
        .map(|(_, rect)| AtomicBitGrid::new(rect.width(), rect.height()))
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
                let radius_sqr = light.radius * light.radius;
                let alpha_test = light.alpha_tested();
                for texel in &texels {
                    let to_light = light.position - texel.position;
                    let distance_sqr = to_light.length_squared();
                    if distance_sqr > radius_sqr {
                        continue;
                    }
                    let distance = distance_sqr.sqrt();
                    let lit = if distance > 0.0 {
                        let direction = to_light / distance;
                        if texel.normal.dot(direction) <= NDOTL_TOLERANCE {
                            continue;
                        }
                        rays += 1;
                        let origin = texel.position + offset;
                        !scene
                            .occluders
                            .occluded(origin, direction, distance, alpha_test)
                    } else {
                        true
                    };
                    visited.set(texel.x, texel.y);
                    if lit {
                        grid.set(texel.x - rect.min_x, texel.y - rect.min_y);
                    }
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
            .map(|(grid, (_, rect))| RegionBits {
                rect: *rect,
                grid: grid.into_grid(),
            })
            .collect(),
        texels,
        rays,
    }
}
