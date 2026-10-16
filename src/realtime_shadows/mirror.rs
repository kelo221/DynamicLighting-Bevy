//! CPU reference for the realtime shadow test in `vsm.wesl` (the smooth terminator and the
//! variance (Chebyshev) test with Henry's light-bleeding cut) and for the face blur kernel in
//! `shadow_kernel.wesl`.

use bevy::math::{Vec2, Vec3};

use crate::shared::math::{inverse_lerp_clamped, smoothstep01};

/// The most taps on each side of the center a face blur takes.
pub const MAX_BLUR_TAPS: i32 = 6;

/// Taps on each side of the center for a face blur of `sigma` texels: 2 at sigma 1.5.
pub fn blur_taps(sigma: f32) -> i32 {
    ((sigma * 4.0 / 3.0).round() as i32).clamp(1, MAX_BLUR_TAPS)
}

/// The face blur's weights for offsets `0..=blur_taps(sigma)`: `exp(-x² / 2σ²)`, normalized
/// so the taps on both sides sum to 1.
pub fn blur_kernel(sigma: f32) -> Vec<f32> {
    let weights: Vec<f32> = (0..=blur_taps(sigma))
        .map(|x| (-((x * x) as f32) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total = weights[0] + 2.0 * weights[1..].iter().sum::<f32>();
    weights.into_iter().map(|w| w / total).collect()
}

/// Depth bias, relative to the distance.
pub const VSM_BIAS: f32 = 0.0075;
/// The light-bleeding cut never goes below this.
pub const MIN_BLEEDING_CUT: f32 = 0.3;
/// The light fades in while the cosine between the normal and the light rises from START to END.
pub const TERMINATOR_START: f32 = -0.1;
pub const TERMINATOR_END: f32 = 0.3;

/// How much light reaches a surface whose normal makes `cos_theta` with the light direction.
pub fn terminator(cos_theta: f32) -> f32 {
    smoothstep01(inverse_lerp_clamped(
        TERMINATOR_START,
        TERMINATOR_END,
        cos_theta,
    ))
}

/// The Chebyshev bound for a receiver at `depth` (distance / radius) given filtered `moments`,
/// with Henry's light-bleeding cut.
pub fn visibility(moments: Vec2, depth: f32) -> f32 {
    let variance = (moments.y - moments.x * moments.x).max(0.0);
    let d = moments.x - depth;
    let p = variance / (variance + d * d).max(1.0e-12);
    let lit = if depth - VSM_BIAS * depth <= moments.x {
        1.0
    } else {
        p
    };
    let cut = MIN_BLEEDING_CUT.max(depth);
    ((lit - cut) / (1.0 - cut).max(1.0e-6)).clamp(0.0, 1.0)
}

/// `realtime_shadow` for a surface `to_light` away from where the cube of a light of `radius`
/// was rendered (light minus surface), whose normal makes `cosine` with the part of the light
/// it faces most, where `sample(direction)` gives the filtered moments in a direction from the
/// light.
pub fn realtime_shadow(
    to_light: Vec3,
    cosine: f32,
    radius: f32,
    sample: impl Fn(Vec3) -> Vec2,
) -> f32 {
    let fade = terminator(cosine);
    if fade <= 0.0 {
        return 0.0;
    }
    fade * visibility(sample(-to_light), to_light.length() / radius)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realtime_shadows::faces::{MAX_AREA_BLUR, POINT_BLUR, SHADOW_CUBEMAP_RESOLUTION};

    const RADIUS: f32 = 9.0;

    /// Unfiltered moments of a light at the origin whose only occluder is a sphere.
    fn sphere_map(center: Vec3, sphere_radius: f32) -> impl Fn(Vec3) -> Vec2 {
        move |direction: Vec3| {
            let direction = direction.normalize();
            let along = center.dot(direction);
            let miss_sqr = center.length_squared() - along * along;
            let hit = along - (sphere_radius * sphere_radius - miss_sqr).max(0.0).sqrt();
            let depth = if miss_sqr > sphere_radius * sphere_radius || hit <= 0.0 {
                1.0
            } else {
                (hit / RADIUS).min(1.0)
            };
            Vec2::new(depth, depth * depth)
        }
    }

    /// `map` blurred like a point light's cubemap faces: the separable kernel around
    /// `direction`.
    fn blurred(map: impl Fn(Vec3) -> Vec2) -> impl Fn(Vec3) -> Vec2 {
        let kernel = blur_kernel(POINT_BLUR);
        let taps = kernel.len() as i32 - 1;
        move |direction: Vec3| {
            let direction = direction.normalize();
            let (u, v) = direction.any_orthonormal_pair();
            let texel = 2.0 / SHADOW_CUBEMAP_RESOLUTION as f32;
            let weight = |i: i32| kernel[i.unsigned_abs() as usize];
            let mut sum = Vec2::ZERO;
            for i in -taps..=taps {
                for j in -taps..=taps {
                    let offset = (u * i as f32 + v * j as f32) * texel;
                    sum += map(direction + offset) * (weight(i) * weight(j));
                }
            }
            sum
        }
    }

    #[test]
    fn the_point_light_kernel_is_five_gaussian_taps() {
        // Sigma 1.5 over 5 taps, the kernel point lights have always used.
        let expected = [0.292_081_7, 0.233_880_76, 0.120_078_38];
        let kernel = blur_kernel(POINT_BLUR);
        assert_eq!(kernel.len(), expected.len());
        for (weight, expected) in kernel.iter().zip(expected) {
            assert!((weight - expected).abs() < 1e-6, "{weight} != {expected}");
        }
        let widest = blur_kernel(MAX_AREA_BLUR);
        assert!(widest.len() as i32 - 1 <= MAX_BLUR_TAPS);
        assert!((widest[0] + 2.0 * widest[1..].iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert!(widest[0] < kernel[0], "wider kernels spread their weight");
    }

    #[test]
    fn the_terminator_is_smooth_and_monotonic() {
        assert_eq!(terminator(-1.0), 0.0);
        assert_eq!(terminator(TERMINATOR_START), 0.0);
        assert_eq!(terminator(TERMINATOR_END), 1.0);
        let samples: Vec<f32> = (0..=200)
            .map(|i| terminator(i as f32 / 100.0 - 1.0))
            .collect();
        for pair in samples.windows(2) {
            assert!(pair[1] >= pair[0]);
            assert!(pair[1] - pair[0] < 0.08);
        }
    }

    #[test]
    fn open_and_blocked_receivers() {
        let empty = |_: Vec3| Vec2::ONE;
        let to_light = Vec3::new(0.0, 4.0, 0.0);
        let shadow = |normal: Vec3, map: &dyn Fn(Vec3) -> Vec2| {
            realtime_shadow(to_light, normal.dot(to_light.normalize()), RADIUS, map)
        };
        assert_eq!(shadow(Vec3::Y, &empty), 1.0);
        assert_eq!(shadow(Vec3::NEG_Y, &empty), 0.0);
        // The receiver sits 4 below the light; a sphere 2 below it blocks the light.
        let blocked = sphere_map(Vec3::new(0.0, -2.0, 0.0), 0.5);
        assert_eq!(shadow(Vec3::Y, &blocked), 0.0);
    }

    /// Around a lit sphere, `(cos θ, visibility from the blurred map)` per degree.
    fn sphere_visibility() -> Vec<(f32, f32)> {
        let center = Vec3::new(4.5, 0.0, 0.0);
        let sphere_radius = 0.3;
        let map = blurred(sphere_map(center, sphere_radius));
        let to_center = -center.normalize();
        (0..=180)
            .map(|step| {
                let angle = (step as f32).to_radians();
                let normal = to_center * angle.cos() + Vec3::Y * angle.sin();
                let to_light = -(center + normal * sphere_radius);
                let cos_theta = normal.dot(to_light.normalize());
                (
                    cos_theta,
                    visibility(map(-to_light), to_light.length() / RADIUS),
                )
            })
            .collect()
    }

    #[test]
    fn a_sphere_fades_out_before_its_shadow_map_cuts_in() {
        let samples = sphere_visibility();
        let faded = samples
            .iter()
            .filter(|(cos_theta, _)| *cos_theta >= TERMINATOR_START);
        assert!(faded.clone().count() > 90);
        for (cos_theta, visibility) in faded {
            assert!(
                *visibility > 0.99,
                "cos {cos_theta}: visibility {visibility}"
            );
        }
        // Further round the map does cut in, which is why the terminator can't start lower.
        assert!(
            samples
                .iter()
                .any(|&(cos_theta, visibility)| cos_theta > -0.6 && visibility < 0.5)
        );
    }
}
