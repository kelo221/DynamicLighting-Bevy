//! Henry's `DistanceCubesGenerate`: per baked light, a 64² × 6 cube of the distance to the
//! nearest static surface, which moving meshes use as a coarse shadow map.
//!
//! Henry renders these with a GPU camera; here every texel is a closest-hit ray from the light
//! through the texel center ([`crate::shared::cube`] conventions, faces `+X −X +Y −Y +Z −Z`).
//! Values are meters (a miss stores the light radius), packed as two f16 per word: texel `2i`
//! in the low half of word `i`, texel `2i + 1` in the high half. Distances are exact in every
//! mode: the `DistanceCubes` filter adds Henry's +0.25 m bias in the shader, so area lights can
//! read true blocker distances. An area light's cube is seen from its center.

use half::f16;
use rayon::prelude::*;

use crate::shared::DynamicGeometryMode;
use crate::shared::cube::{CUBE_FACE_COUNT, texel_direction};

use super::input::{BakeError, BakeLight};
use super::occluder::RayCaster;
use super::progress::BakeProgress;

/// Texels per side of a distance cube face.
pub const DISTANCE_CUBE_RESOLUTION: u32 = 64;
/// Packs two distances into one word, `lower` in bits 0–15.
pub fn pack_f16_pair(lower: f32, upper: f32) -> u32 {
    u32::from(f16::from_f32(upper).to_bits()) << 16 | u32::from(f16::from_f32(lower).to_bits())
}

/// The distance cubes of `lights` (BVH order), concatenated. Empty for `LightingOnly`.
pub fn bake_distance_cubes(
    lights: &[&BakeLight],
    caster: &(impl RayCaster + ?Sized),
    mode: DynamicGeometryMode,
    progress: &BakeProgress,
) -> Result<Vec<u32>, BakeError> {
    if mode == DynamicGeometryMode::LightingOnly {
        return Ok(Vec::new());
    }
    let resolution = DISTANCE_CUBE_RESOLUTION;
    let faces: Vec<Vec<u32>> = (0..lights.len() as u32 * CUBE_FACE_COUNT)
        .into_par_iter()
        .map(|job| {
            if progress.is_cancelled() {
                return Vec::new();
            }
            let light = lights[(job / CUBE_FACE_COUNT) as usize];
            let face = job % CUBE_FACE_COUNT;
            let distances: Vec<f32> = (0..resolution * resolution)
                .map(|texel| {
                    let direction =
                        texel_direction(face, texel % resolution, texel / resolution, resolution);
                    caster
                        .closest_hit(
                            light.position,
                            direction,
                            light.radius,
                            light.alpha_tested(),
                        )
                        .map_or(light.radius, |hit| hit.distance)
                })
                .collect();
            progress.advance(1);
            distances
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[lower, upper]| pack_f16_pair(lower, upper))
                .collect()
        })
        .collect();
    progress.check()?;
    Ok(faces.concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::mirror::{DISTANCE_CUBE_WORDS, distance_cube_value};
    use crate::bake::occluder::SurfaceHit;
    use crate::shared::cube::{direction_to_texel, texel_index};
    use bevy::math::Vec3;

    /// A floor plane at y = -1 under everything.
    struct Floor;

    impl RayCaster for Floor {
        fn occluded(&self, _: Vec3, _: Vec3, _: f32, _: bool) -> bool {
            unreachable!()
        }

        fn closest_hit(&self, origin: Vec3, dir: Vec3, max: f32, _: bool) -> Option<SurfaceHit> {
            let t = (-1.0 - origin.y) / dir.y;
            (dir.y < 0.0 && t <= max).then_some(SurfaceHit {
                distance: t,
                occluder: 0,
                barycentrics: Vec3::ONE / 3.0,
            })
        }
    }

    fn light(radius: f32) -> BakeLight {
        BakeLight {
            id: crate::shared::BakeId(1),
            position: Vec3::ZERO,
            radius,
            illumination: Default::default(),
            transparency: Default::default(),
            bounce: Default::default(),
            shape: crate::light::LightShape::Point,
        }
    }

    #[test]
    fn packs_low_half_first() {
        let word = pack_f16_pair(1.0, 2.0);
        assert_eq!(word & 0xFFFF, u32::from(f16::from_f32(1.0).to_bits()));
        assert_eq!(word >> 16, u32::from(f16::from_f32(2.0).to_bits()));
    }

    #[test]
    fn cubes_store_hit_distances_in_texel_order() {
        let light = light(4.0);
        let progress = BakeProgress::new();
        let cubes = bake_distance_cubes(
            &[&light, &light],
            &Floor,
            DynamicGeometryMode::Angular,
            &progress,
        )
        .unwrap();
        assert_eq!(cubes.len(), 2 * DISTANCE_CUBE_WORDS as usize);
        // Straight down hits the floor at 1 m; straight up misses and stores the radius.
        for (direction, expected) in [(Vec3::NEG_Y, 1.0), (Vec3::Y, 4.0), (Vec3::X, 4.0)] {
            let (face, x, y) = direction_to_texel(direction, DISTANCE_CUBE_RESOLUTION);
            let index = texel_index(face, x, y, DISTANCE_CUBE_RESOLUTION);
            for slot in 0..2 {
                let value = distance_cube_value(&cubes, slot, index);
                assert!((value - expected).abs() < 0.01, "{direction}: {value}");
            }
        }
        // A slanted texel: distance 1 / cos(angle) from the light to the floor.
        let direction = texel_direction(3, 10, 20, DISTANCE_CUBE_RESOLUTION);
        let index = texel_index(3, 10, 20, DISTANCE_CUBE_RESOLUTION);
        let expected = 1.0 / -direction.y;
        assert!((distance_cube_value(&cubes, 1, index) - expected).abs() < expected * 1e-3);
    }

    #[test]
    fn distance_cubes_mode_stores_exact_distances_and_lighting_only_bakes_nothing() {
        let light = light(4.0);
        let progress = BakeProgress::new();
        let cubes = bake_distance_cubes(
            &[&light],
            &Floor,
            DynamicGeometryMode::DistanceCubes,
            &progress,
        )
        .unwrap();
        let (face, x, y) = direction_to_texel(Vec3::NEG_Y, DISTANCE_CUBE_RESOLUTION);
        let value = distance_cube_value(&cubes, 0, texel_index(face, x, y, 64));
        assert!((value - 1.0).abs() < 0.01);
        let none = bake_distance_cubes(
            &[&light],
            &Floor,
            DynamicGeometryMode::LightingOnly,
            &progress,
        );
        assert_eq!(none, Ok(Vec::new()));
    }
}
