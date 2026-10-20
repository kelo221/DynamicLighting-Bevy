//! Single-bounce lighting: Henry's `BounceTriangle` gather against a photon cube per light.
//!
//! Henry renders photon cubes on the GPU (1024² per face, back faces culled). Here each texel
//! is a CPU closest-hit ray from the light ([`BakeOptions::photon_cube_resolution`] per face).
//! Hits on the side of a surface facing away from the light store no normal, so unlit back
//! faces bounce nothing.

use bevy::math::Vec3;
use rayon::prelude::*;

use crate::shared::cube::{CUBE_FACE_COUNT, direction_to_texel, texel_direction, texel_index};

use super::bits::TexelRect;
use super::bounce_map::BounceMap;
use super::input::{BakeLight, BakeOptions};
use super::mesh_bake::{RAY_OFFSET, Scene, TriangleSetup};
use super::mesh_builder::PreparedMesh;
use super::mesh_texels::MeshTexels;
use super::occluder::{Occluders, RayCaster};
use super::progress::BakeProgress;
use super::random;
use super::texels::SurfaceTexel;

/// The first surface seen from a light through one cube texel.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Photon {
    /// Distance from the light to the hit, nudged towards the light and off the surface like
    /// Henry's photon shader; the light radius on a miss.
    distance: f32,
    /// Surface normal quantized to 8 bits per axis (Henry stores 8 bits too); zero on a miss.
    normal: [i8; 3],
}

/// Per bounce light: the first surface seen from the light in every cube texel direction.
pub struct PhotonCube {
    resolution: u32,
    photons: Vec<Photon>,
}

impl PhotonCube {
    fn build(
        light: &BakeLight,
        occluders: &Occluders,
        resolution: u32,
        progress: &BakeProgress,
    ) -> Self {
        let rows: Vec<Vec<Photon>> = (0..CUBE_FACE_COUNT * resolution)
            .into_par_iter()
            .map(|row| {
                if progress.is_cancelled() {
                    return Vec::new();
                }
                let (face, y) = (row / resolution, row % resolution);
                (0..resolution)
                    .map(|x| photon(light, occluders, texel_direction(face, x, y, resolution)))
                    .collect()
            })
            .collect();
        Self {
            resolution,
            photons: rows.concat(),
        }
    }

    /// The photon in `direction`: its world position and unit normal (zero when none).
    fn sample(&self, light: Vec3, direction: Vec3) -> (Vec3, Vec3) {
        let (face, x, y) = direction_to_texel(direction, self.resolution);
        let index = texel_index(face, x, y, self.resolution) as usize;
        let Some(photon) = self.photons.get(index) else {
            return (light, Vec3::ZERO);
        };
        let normal = Vec3::from(photon.normal.map(|n| f32::from(n) / 127.0));
        (
            light + direction * photon.distance,
            normal.normalize_or_zero(),
        )
    }
}

fn photon(light: &BakeLight, occluders: &Occluders, direction: Vec3) -> Photon {
    let alpha_test = light.alpha_tested();
    let Some(hit) = occluders.closest_hit(light.position, direction, light.radius, alpha_test)
    else {
        return Photon {
            distance: light.radius,
            normal: [0; 3],
        };
    };
    let normal = occluders.surface_normal(&hit);
    let point = light.position + direction * hit.distance;
    let bias = (hit.distance * 0.001).max(0.001);
    let nudged = point - direction * bias + normal * bias;
    let lit_side = occluders.face_normal(&hit).dot(direction) < 0.0;
    Photon {
        distance: (nudged - light.position).length(),
        normal: if lit_side {
            normal.to_array().map(|n| (n * 127.0).round() as i8)
        } else {
            [0; 3]
        },
    }
}

/// Photon cubes for every light that bakes bounce, by BVH slot.
pub fn photon_cubes(
    lights: &[&BakeLight],
    occluders: &Occluders,
    options: &BakeOptions,
    progress: &BakeProgress,
) -> Vec<Option<PhotonCube>> {
    lights
        .iter()
        .map(|light| {
            if !light.bakes_bounce(options) || progress.is_cancelled() {
                return None;
            }
            let resolution = options.photon_cube_resolution;
            let cube = PhotonCube::build(light, occluders, resolution, progress);
            progress.advance(1);
            Some(cube)
        })
        .collect()
}

/// Bounce grids for one mesh: `result[t][k]` is the padded bounce grid of light `k` of
/// triangle `t`, when that light bounces onto it. Also returns the bounce rays traced.
pub fn bake_mesh_bounce(
    scene: &Scene,
    prepared: &PreparedMesh,
    setups: &[TriangleSetup],
    mesh_texels: &MeshTexels,
) -> (Vec<Vec<Option<Vec<f32>>>>, u64) {
    if !scene.bounce_in_scene {
        return (Vec::new(), 0);
    }
    let mut result: Vec<Vec<Option<Vec<f32>>>> =
        setups.iter().map(|s| vec![None; s.lights.len()]).collect();
    let mut rays = 0;
    for (slot, cube) in scene.photons.iter().enumerate() {
        let Some(cube) = cube else {
            continue;
        };
        if scene.progress.is_cancelled() {
            break;
        }
        // (triangle, entry index) pairs of this light, in triangle order.
        let receivers: Vec<(usize, usize)> = setups
            .iter()
            .enumerate()
            .filter_map(|(t, s)| Some((t, s.lights.iter().position(|&l| l == slot as u32)?)))
            .collect();
        let Some(crops) = receivers
            .iter()
            .map(|&(t, _)| setups[t].crop)
            .reduce(TexelRect::union)
        else {
            continue;
        };
        let light = scene.lights[slot];
        let gathered: Vec<Gathered> = receivers
            .par_iter()
            .map(|&(t, _)| {
                let texels = mesh_texels.of(prepared, t, setups[t].face_normal);
                gather_triangle(scene, &texels, &setups[t], prepared, light, cube)
            })
            .collect();
        // Later triangles overwrite shared texels, as in Henry's sequential loop.
        let mut map = BounceMap::new(prepared.resolution, crops);
        for (texels, count) in gathered {
            rays += count;
            for (x, y, value) in texels {
                map.set(x, y, value);
            }
        }
        map.dilate();
        map.blur();
        for &(t, k) in &receivers {
            result[t][k] = map.crop(setups[t].crop);
        }
    }
    (result, rays)
}

/// One triangle's gathered `(x, y, value)` texels and the rays traced for them.
type Gathered = (Vec<(u32, u32, f32)>, u64);

/// Henry's per-texel gather: `samples` directions spreading from the light→texel direction
/// (0.1 → 1.0 of the way to a random direction), each looked up in the photon cube; visible
/// photons add `max(n_s·d, 0) · max(n_p·−d, 0) · intensity`, averaged over all samples.
fn gather_triangle(
    scene: &Scene,
    texels: &[SurfaceTexel],
    setup: &TriangleSetup,
    prepared: &PreparedMesh,
    light: &BakeLight,
    cube: &PhotonCube,
) -> Gathered {
    if setup.face_normal == Vec3::ZERO {
        return (Vec::new(), 0);
    }
    let samples = light.bounce.samples.max(2);
    let spread_step = 0.9 / (samples - 1) as f32;
    let alpha_test = light.alpha_tested();
    let normal = setup.face_normal;
    let mut rays = 0u64;
    let mut out = Vec::with_capacity(texels.len());
    for texel in texels {
        let to_texel = texel.position - light.position;
        if to_texel.length_squared() > light.radius * light.radius {
            continue;
        }
        let direction = to_texel.normalize_or(Vec3::Y);
        let origin = texel.position + normal * RAY_OFFSET;
        let mut rng = random::stream(&[
            scene.options.seed,
            prepared.mesh.id.0,
            light.id.0,
            u64::from(texel.x),
            u64::from(texel.y),
        ]);
        let mut sum = 0.0;
        for i in 0..samples {
            let spread = 0.1 + i as f32 * spread_step;
            let random = random::on_unit_sphere(&mut rng);
            let sample = direction.slerp(random, spread).normalize_or(direction);
            let (photon, photon_normal) = cube.sample(light.position, sample);
            let to_photon = photon - origin;
            let distance = to_photon.length();
            // Photons without a normal add nothing, so their visibility ray is skipped.
            if photon_normal == Vec3::ZERO || distance <= 0.0 {
                continue;
            }
            let d = to_photon / distance;
            rays += 1;
            if !scene.occluders.occluded(origin, d, distance, alpha_test) {
                sum += normal.dot(d).max(0.0) * photon_normal.dot(-d).max(0.0);
            }
        }
        out.push((
            texel.x,
            texel.y,
            sum * light.bounce.intensity / samples as f32,
        ));
    }
    (out, rays)
}
