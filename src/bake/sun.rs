//! Sun shadows: per texel, shadow rays towards the sun's disk. A sun without a disk takes one
//! ray, like Henry's point lights; a soft sun probes its disk and spreads rays over it like an
//! area light, every direction of the disk sending the same light.

use bevy::math::{Vec2, Vec3};

use crate::effects::EffectRng;
use crate::light::SunDisk;

use super::area::{AreaSampling, TexelVisibility, adaptive_share};
use super::mesh_bake::NDOTL_TOLERANCE;
use super::spread::{concentric_disk, spread};

/// The disk's center and four points of its rim, as disk coordinates.
const DISK_PROBES: [Vec2; 5] = [Vec2::ZERO, Vec2::X, Vec2::Y, Vec2::NEG_X, Vec2::NEG_Y];

/// How much of `sun` reaches a texel with unit `normal`, by [`adaptive_share`] over the disk;
/// each ray is weighted by the receiver cosine with Henry's facing tolerance. `None` when the
/// whole disk is below the texel's horizon. `occluded(direction)` traces one ray from the texel
/// to the end of the scene; `rng` shifts the spread and is only drawn from for a soft sun.
pub fn sun_visibility(
    sun: &SunDisk,
    normal: Vec3,
    sampling: AreaSampling,
    rng: &mut EffectRng,
    mut occluded: impl FnMut(Vec3) -> bool,
) -> Option<TexelVisibility> {
    let toward = sun.toward();
    let radius = sun.angular_radius();
    if radius <= 0.0 {
        if normal.dot(toward) <= NDOTL_TOLERANCE {
            return None;
        }
        let lit = !occluded(toward);
        return Some(TexelVisibility {
            fraction: if lit { 1.0 } else { 0.0 },
            rays: 1,
        });
    }
    let (side, up) = toward.any_orthonormal_pair();
    let tan = radius.tan();
    let mut rays = 0;
    let trace = |disk: Vec2| {
        let direction = (toward + (side * disk.x + up * disk.y) * tan).normalize();
        let receiver = normal.dot(direction) - NDOTL_TOLERANCE;
        if receiver <= 0.0 {
            return None;
        }
        rays += 1;
        Some((receiver, !occluded(direction)))
    };
    let offset = Vec2::new(rng.next_f32(), rng.next_f32());
    let spread_point = |taken| concentric_disk(spread(taken, offset, true));
    let fraction = adaptive_share(&DISK_PROBES, sampling, spread_point, trace);
    fraction.map(|fraction| TexelVisibility { fraction, rays })
}

#[cfg(test)]
mod tests {
    use bevy::math::Quat;

    use super::*;
    use crate::bake::random;
    use crate::light::DynamicLight;

    /// A sun `degrees` wide, straight above.
    fn overhead(degrees: f32) -> SunDisk {
        SunDisk {
            rotation: Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2),
            angular_diameter: degrees,
        }
    }

    fn visibility(
        sun: &SunDisk,
        normal: Vec3,
        minimum: u32,
        blocked: impl FnMut(Vec3) -> bool,
    ) -> Option<TexelVisibility> {
        let mut rng = random::stream(&[4, 5, 6]);
        let sampling = AreaSampling {
            samples: 64,
            minimum,
        };
        sun_visibility(sun, normal, sampling, &mut rng, blocked)
    }

    #[test]
    fn suns_without_a_disk_take_one_hard_ray() {
        let sun = overhead(0.0);
        let open = visibility(&sun, Vec3::Y, 16, |_| false).unwrap();
        assert_eq!((open.fraction, open.rays), (1.0, 1));
        let covered = visibility(&sun, Vec3::Y, 16, |_| true).unwrap();
        assert_eq!((covered.fraction, covered.rays), (0.0, 1));
        assert_eq!(visibility(&sun, Vec3::NEG_Y, 16, |_| false), None);
        let grazing = Vec3::new(1.0, -0.05, 0.0).normalize();
        assert!(
            visibility(&sun, grazing, 0, |_| false).is_some(),
            "tolerance"
        );
    }

    #[test]
    fn soft_suns_stop_after_the_probes_in_the_open() {
        let sun = overhead(DynamicLight::SUN_ANGULAR_DIAMETER);
        let open = visibility(&sun, Vec3::Y, 0, |_| false).unwrap();
        assert_eq!((open.fraction, open.rays), (1.0, 5));
        let open = visibility(&sun, Vec3::Y, 16, |_| false).unwrap();
        assert_eq!((open.fraction, open.rays), (1.0, 5 + 16));
    }

    #[test]
    fn an_edge_across_the_disk_gives_a_penumbra() {
        // An edge through the disk's center hides the half of it towards −X.
        let sun = overhead(4.0);
        let half = visibility(&sun, Vec3::Y, 16, |direction| direction.x < 0.0).unwrap();
        assert_eq!(half.rays, 5 + 64, "a penumbra takes every sample");
        assert!((half.fraction - 0.5).abs() < 0.06, "{}", half.fraction);
        // An edge across a quarter of the disk's width leaves more than 3/4 lit.
        let edge = (-0.5 * 2.0_f32.to_radians()).tan();
        let side = visibility(&sun, Vec3::Y, 16, |d| d.x / d.y < edge).unwrap();
        assert!(
            side.fraction > 0.75 && side.fraction < 0.95,
            "{}",
            side.fraction
        );
    }

    #[test]
    fn disk_rays_stay_inside_the_disk() {
        let sun = overhead(4.0);
        let limit = (2.0_f32.to_radians()).cos() - 1e-6;
        let mut widest = 1.0_f32;
        visibility(&sun, Vec3::Y, 64, |direction| {
            widest = widest.min(direction.dot(Vec3::Y));
            false
        });
        assert!(widest >= limit, "{widest} < {limit}");
        assert!(
            widest < 2.0_f32.to_radians().cos() + 1e-4,
            "the rim is reached"
        );
    }
}
