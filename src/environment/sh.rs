//! The sky's diffuse light as spherical harmonics of bands 0 to 2: projection of an
//! equirectangular image and the cosine convolution that turns radiance into irradiance
//! (Ramamoorthi and Hanrahan, "An Efficient Representation for Irradiance Environment Maps",
//! 2001). `environment/shaders/sky.wesl` evaluates the same basis.

use bevy::math::Vec3;
use rayon::prelude::*;

use super::equirect::{Equirect, uv_to_direction};
use crate::shared::GpuSkyIrradiance;

/// Coefficients of bands 0 to 2.
pub const SH_COUNT: usize = 9;

/// The real spherical harmonics at unit direction `d`, in the order 1, y, z, x, xy, yz,
/// 3z² − 1, xz, x² − y² (with their normalization constants).
pub fn basis(d: Vec3) -> [f32; SH_COUNT] {
    [
        0.282_095,
        0.488_603 * d.y,
        0.488_603 * d.z,
        0.488_603 * d.x,
        1.092_548 * d.x * d.y,
        1.092_548 * d.y * d.z,
        0.315_392 * (3.0 * d.z * d.z - 1.0),
        1.092_548 * d.x * d.z,
        0.546_274 * (d.x * d.x - d.y * d.y),
    ]
}

/// The coefficients of the image's radiance: every texel weighted by its solid angle. Rows are
/// summed in parallel and then in order, so the result doesn't depend on the thread count.
pub fn project(image: &Equirect) -> [Vec3; SH_COUNT] {
    let rows: Vec<[Vec3; SH_COUNT]> = (0..image.height)
        .into_par_iter()
        .map(|y| {
            let solid_angle = image.texel_solid_angle(y);
            let v = (y as f32 + 0.5) / image.height as f32;
            let mut row = [Vec3::ZERO; SH_COUNT];
            for x in 0..image.width {
                let u = (x as f32 + 0.5) / image.width as f32;
                let direction = uv_to_direction(bevy::math::Vec2::new(u, v));
                let radiance = image.texel(x, y) * solid_angle;
                for (sum, value) in row.iter_mut().zip(basis(direction)) {
                    *sum += radiance * value;
                }
            }
            row
        })
        .collect();
    rows.into_iter()
        .fold([Vec3::ZERO; SH_COUNT], |mut total, row| {
            for (sum, value) in total.iter_mut().zip(row) {
                *sum += value;
            }
            total
        })
}

/// Radiance coefficients convolved with the clamped cosine and divided by π: evaluated at a
/// normal they give the irradiance over π there, which is what a white Lambert surface shows.
pub fn irradiance_over_pi(radiance: [Vec3; SH_COUNT]) -> [Vec3; SH_COUNT] {
    // The cosine lobe's band factors π, 2π/3 and π/4, over π.
    const BANDS: [f32; SH_COUNT] = [
        1.0,
        2.0 / 3.0,
        2.0 / 3.0,
        2.0 / 3.0,
        0.25,
        0.25,
        0.25,
        0.25,
        0.25,
    ];
    let mut out = radiance;
    for (coefficient, band) in out.iter_mut().zip(BANDS) {
        *coefficient *= band;
    }
    out
}

/// The coefficients evaluated at unit direction `n`, clamped at zero like the shader.
pub fn evaluate(coefficients: &[Vec3; SH_COUNT], n: Vec3) -> Vec3 {
    let value = coefficients
        .iter()
        .zip(basis(n))
        .fold(Vec3::ZERO, |sum, (c, y)| sum + *c * y);
    value.max(Vec3::ZERO)
}

/// The sky's diffuse light for the light buffer.
pub fn to_gpu(coefficients: &[Vec3; SH_COUNT]) -> GpuSkyIrradiance {
    GpuSkyIrradiance {
        sh: coefficients.map(|c| [c.x, c.y, c.z, 0.0]),
    }
}

/// The irradiance over π at `n` from `image`, integrated texel by texel. Slow; for tests.
#[cfg(test)]
pub(super) fn brute_force_irradiance_over_pi(image: &Equirect, n: Vec3) -> Vec3 {
    use core::f32::consts::PI;
    let mut sum = Vec3::ZERO;
    for y in 0..image.height {
        let solid_angle = image.texel_solid_angle(y);
        for x in 0..image.width {
            let uv = bevy::math::Vec2::new(
                (x as f32 + 0.5) / image.width as f32,
                (y as f32 + 0.5) / image.height as f32,
            );
            let cosine = uv_to_direction(uv).dot(n).max(0.0);
            sum += image.texel(x, y) * (cosine * solid_angle);
        }
    }
    sum / PI
}

#[cfg(test)]
mod tests {
    use super::*;

    fn irradiance(image: &Equirect) -> [Vec3; SH_COUNT] {
        irradiance_over_pi(project(image))
    }

    #[test]
    fn a_uniform_sky_lights_every_side_alike() {
        let image = Equirect::from_fn(64, 32, |_| Vec3::new(1.0, 0.5, 0.25));
        let sh = irradiance(&image);
        for n in [Vec3::X, Vec3::NEG_Y, Vec3::new(0.6, 0.0, 0.8)] {
            let e = evaluate(&sh, n);
            assert!(e.abs_diff_eq(Vec3::new(1.0, 0.5, 0.25), 2e-3), "{n} {e}");
        }
    }

    #[test]
    fn matches_the_integral_for_a_smooth_sky() {
        let image = Equirect::from_fn(128, 64, |d| {
            Vec3::new(1.0 + d.y, 0.5 + 0.5 * d.x * d.x, 0.2 + 0.3 * d.z.max(0.0))
        });
        let sh = irradiance(&image);
        for n in [Vec3::Y, Vec3::NEG_Y, Vec3::X, Vec3::new(0.0, 0.6, -0.8)] {
            let exact = brute_force_irradiance_over_pi(&image, n);
            let e = evaluate(&sh, n);
            assert!((e - exact).abs().max_element() < 0.02, "{n} {e} {exact}");
        }
    }

    #[test]
    fn a_bright_upper_half_lights_from_above() {
        let image = Equirect::from_fn(128, 64, |d| Vec3::splat(f32::from(d.y > 0.0)));
        let sh = irradiance(&image);
        let up = evaluate(&sh, Vec3::Y).x;
        let side = evaluate(&sh, Vec3::X).x;
        let down = evaluate(&sh, Vec3::NEG_Y).x;
        // Exactly 1, 1/2 and 0; three bands ring a little.
        assert!((up - 1.0).abs() < 0.06 && (side - 0.5).abs() < 0.01 && down < 0.06);
    }

    #[test]
    fn gpu_coefficients_keep_the_channels() {
        let mut sh = [Vec3::ZERO; SH_COUNT];
        sh[4] = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(to_gpu(&sh).sh[4], [1.0, 2.0, 3.0, 0.0]);
    }
}
