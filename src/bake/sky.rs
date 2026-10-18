//! Sky shadows: per texel, how much of the sky it sees. Rays leave over the hemisphere around the
//! texel's normal with a cosine density, so the share that leaves the scene is the share of a
//! uniform sky's light a Lambert surface there receives. It doesn't depend on the sky's image or
//! turn, which is why a baked sky may change both.

use bevy::math::{Vec2, Vec3};

use crate::effects::EffectRng;

use super::area::TexelVisibility;
use super::spread::{concentric_disk, spread};

/// Shadow bits per texel of a sky light: 256 levels, because sky shadows are slow gradients
/// over large areas where 16 levels would show bands.
pub const SKY_SHADOW_BITS: u32 = 8;

/// The direction of point `square` of the unit square over the hemisphere around unit `normal`,
/// with a cosine density (Malley's method: a point of the concentric disk lifted onto the
/// hemisphere).
pub fn cosine_direction(normal: Vec3, square: Vec2) -> Vec3 {
    let disk = concentric_disk(square);
    let up = (1.0 - disk.length_squared()).max(0.0).sqrt();
    let (side, across) = normal.any_orthonormal_pair();
    (side * disk.x + across * disk.y + normal * up).normalize()
}

/// How much of the sky a texel with unit `normal` sees: the share of `samples` rays that leave
/// the scene, sent through the Sobol (0, 2) points shifted by `rng`. `occluded(direction)`
/// traces one ray from the texel to the end of the scene.
pub fn sky_visibility(
    normal: Vec3,
    samples: u32,
    rng: &mut EffectRng,
    mut occluded: impl FnMut(Vec3) -> bool,
) -> TexelVisibility {
    let offset = Vec2::new(rng.next_f32(), rng.next_f32());
    let open = (0..samples)
        .filter(|&taken| !occluded(cosine_direction(normal, spread(taken, offset, true))))
        .count();
    TexelVisibility {
        fraction: open as f32 / samples.max(1) as f32,
        rays: samples,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::random;

    fn visibility(normal: Vec3, samples: u32, blocked: impl FnMut(Vec3) -> bool) -> f32 {
        sky_visibility(normal, samples, &mut random::stream(&[1, 2, 3]), blocked).fraction
    }

    #[test]
    fn directions_follow_the_cosine() {
        let normal = Vec3::new(0.3, 0.8, -0.5).normalize();
        let count = 4096;
        let mut cosine = 0.0;
        for index in 0..count {
            let direction = cosine_direction(normal, spread(index, Vec2::splat(0.3), true));
            assert!((direction.length() - 1.0).abs() < 1e-5);
            assert!(direction.dot(normal) >= 0.0);
            cosine += direction.dot(normal);
        }
        // The mean cosine of a cosine-weighted hemisphere is 2/3.
        let mean = cosine / count as f32;
        assert!((mean - 2.0 / 3.0).abs() < 2e-3, "{mean}");
    }

    #[test]
    fn open_and_covered_texels() {
        assert_eq!(visibility(Vec3::Y, 64, |_| false), 1.0);
        assert_eq!(visibility(Vec3::Y, 64, |_| true), 0.0);
        let half = visibility(Vec3::Y, 64, |direction| direction.x < 0.0);
        assert!((half - 0.5).abs() < 0.04, "{half}");
    }

    #[test]
    fn a_raised_horizon_leaves_the_cosine_squared_of_its_height() {
        // Blocking everything lower than 30° above the horizon leaves sin² 60° = 3/4.
        let low = 30.0_f32.to_radians().sin();
        let open = visibility(Vec3::Y, 256, |direction| direction.y < low);
        assert!((open - 0.75).abs() < 0.02, "{open}");
    }
}
