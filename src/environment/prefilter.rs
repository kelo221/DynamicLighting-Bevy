//! The sky's specular light: a cube map whose mip level `m` holds the sky's radiance averaged
//! over the GGX lobe of perceptual roughness `m / (levels − 1)`, which is how the lit shader
//! picks a level (the prefiltered-radiance half of Karis's split sum, "Real Shading in Unreal
//! Engine 4", 2013). Samples are drawn by importance and read from the pyramid level that
//! matches their footprint (Křivánek and Colbert, "Real-time Shading with Filtered Importance
//! Sampling", 2008), so a few dozen per texel stay smooth. The sample pattern is the same
//! Hammersley set for every texel: the result doesn't depend on the thread count.

use core::f32::consts::{PI, TAU};

use bevy::math::{Vec2, Vec3};
use rayon::prelude::*;

use super::equirect::Pyramid;
use crate::shared::cube::{CUBE_FACE_COUNT, texel_direction};

/// Texels per side of the specular cube's first level.
pub const SPECULAR_RESOLUTION: u32 = 128;
/// Mip levels of the specular cube: 128² down to 4², roughness 0 to 1.
pub const SPECULAR_LEVELS: u32 = 6;
/// Importance samples per texel.
pub const SPECULAR_SAMPLES: u32 = 64;

/// The prefiltered cube, before it becomes an image.
#[derive(Clone, Debug, PartialEq)]
pub struct SpecularCube {
    /// Texels per side of level 0.
    pub resolution: u32,
    /// Per level, every face's texels in `cube::texel_index` order.
    pub levels: Vec<Vec<Vec3>>,
}

impl SpecularCube {
    /// Texels per side of `level`.
    pub fn level_resolution(&self, level: usize) -> u32 {
        (self.resolution >> level).max(1)
    }
}

/// The perceptual roughness level `level` of `levels` is filtered for.
pub fn level_roughness(level: u32, levels: u32) -> f32 {
    if levels <= 1 {
        0.0
    } else {
        level as f32 / (levels - 1) as f32
    }
}

/// Prefilters the sky in `pyramid` into a cube of `levels` levels starting at `resolution`.
pub fn prefilter(pyramid: &Pyramid, resolution: u32, levels: u32, samples: u32) -> SpecularCube {
    let filtered = (0..levels)
        .map(|level| {
            let size = (resolution >> level).max(1);
            let roughness = level_roughness(level, levels);
            let texel_solid_angle = 4.0 * PI / (CUBE_FACE_COUNT * size * size) as f32;
            (0..CUBE_FACE_COUNT * size * size)
                .into_par_iter()
                .map(|index| {
                    let (face, rest) = (index / (size * size), index % (size * size));
                    let n = texel_direction(face, rest % size, rest / size, size);
                    prefiltered(pyramid, n, roughness, samples, texel_solid_angle)
                })
                .collect()
        })
        .collect();
    SpecularCube {
        resolution,
        levels: filtered,
    }
}

/// The GGX-weighted radiance around unit direction `n`, seen along `n` (the split sum's
/// assumption), never sharper than an output texel of `texel_solid_angle`.
pub fn prefiltered(
    pyramid: &Pyramid,
    n: Vec3,
    roughness: f32,
    samples: u32,
    texel_solid_angle: f32,
) -> Vec3 {
    let source = pyramid.base_texel_solid_angle();
    let footprint_lod = |solid_angle: f32| 0.5 * (solid_angle / source).log2();
    let texel_lod = footprint_lod(texel_solid_angle).max(0.0);
    let alpha = roughness * roughness;
    if alpha < 1.0e-4 {
        return pyramid.sample(n, texel_lod);
    }
    let (tangent, bitangent) = n.any_orthonormal_pair();
    let mut sum = Vec3::ZERO;
    let mut weight = 0.0;
    for i in 0..samples {
        let (local, n_dot_h) = ggx_half_vector(hammersley(i, samples), alpha);
        let h = tangent * local.x + bitangent * local.y + n * local.z;
        let l = h * (2.0 * n_dot_h) - n;
        let n_dot_l = n.dot(l);
        if n_dot_l <= 0.0 {
            continue;
        }
        // Seen along n, h·v = n·h, so the pdf of l is D(h) / 4.
        let pdf = ggx_distribution(n_dot_h, alpha) / 4.0;
        let lod = (footprint_lod(1.0 / (samples as f32 * pdf)) + 1.0).max(texel_lod);
        sum += pyramid.sample(l.normalize(), lod) * n_dot_l;
        weight += n_dot_l;
    }
    if weight > 0.0 {
        sum / weight
    } else {
        pyramid.sample(n, texel_lod)
    }
}

/// Point `i` of `count` of the Hammersley set in `[0, 1)²`.
pub fn hammersley(i: u32, count: u32) -> Vec2 {
    Vec2::new(
        i as f32 / count as f32,
        i.reverse_bits() as f32 * (1.0 / 4_294_967_296.0),
    )
}

/// A GGX-distributed half vector around +Z for `xi` and α = roughness², and its z.
fn ggx_half_vector(xi: Vec2, alpha: f32) -> (Vec3, f32) {
    let a2 = alpha * alpha;
    let cos_theta = ((1.0 - xi.x) / (1.0 + (a2 - 1.0) * xi.x)).sqrt();
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = TAU * xi.y;
    let h = Vec3::new(sin_theta * phi.cos(), sin_theta * phi.sin(), cos_theta);
    (h, cos_theta)
}

/// GGX normal distribution with α = roughness², as `surface/shaders/brdf.wesl`.
fn ggx_distribution(n_dot_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let denominator = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    a2 / (PI * denominator * denominator).max(1.0e-7)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::equirect::Equirect;

    /// The mean over the sphere: texels weighted by their solid angle.
    fn mean(texels: &[Vec3], size: u32) -> Vec3 {
        let mut sum = Vec3::ZERO;
        let mut weight = 0.0;
        for (index, texel) in texels.iter().enumerate() {
            let rest = index as u32 % (size * size);
            let s = ((rest % size) as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let t = ((rest / size) as f32 + 0.5) / size as f32 * 2.0 - 1.0;
            let solid_angle = (1.0 + s * s + t * t).powf(-1.5);
            sum += *texel * solid_angle;
            weight += solid_angle;
        }
        sum / weight
    }

    #[test]
    fn a_uniform_sky_stays_uniform() {
        let pyramid = Pyramid::new(Equirect::from_fn(64, 32, |_| Vec3::new(2.0, 1.0, 0.5)));
        let cube = prefilter(&pyramid, 8, 4, 32);
        assert_eq!(cube.levels.len(), 4);
        for (level, texels) in cube.levels.iter().enumerate() {
            let size = cube.level_resolution(level);
            assert_eq!(texels.len(), (6 * size * size) as usize);
            for texel in texels {
                assert!(
                    texel.abs_diff_eq(Vec3::new(2.0, 1.0, 0.5), 1e-3),
                    "{level} {texel}"
                );
            }
        }
    }

    #[test]
    fn rough_levels_spread_a_bright_spot() {
        let spot = |d: Vec3| Vec3::splat(if d.y > 0.95 { 50.0 } else { 0.1 });
        let pyramid = Pyramid::new(Equirect::from_fn(128, 64, spot));
        let cube = prefilter(&pyramid, 16, 5, 64);
        let side = Vec3::new(1.0, 1.0, 0.0).normalize();
        let at = |level: u32, d: Vec3| prefiltered(&pyramid, d, level_roughness(level, 5), 64, 0.0);
        assert!(at(0, Vec3::Y).x > 40.0, "a mirror sees the spot");
        assert!(at(0, side).x < 0.2, "a mirror beside it doesn't");
        assert!(at(2, side).x > at(1, side).x, "rougher reaches further");
        assert!(at(4, Vec3::Y).x < at(1, Vec3::Y).x, "and dims the peak");
        // The lobes are normalized, so every level keeps about the sky's mean radiance.
        let sky = 0.1 + 49.9 * 0.025;
        for (level, texels) in cube.levels.iter().enumerate().skip(1) {
            let m = mean(texels, cube.level_resolution(level)).x;
            assert!((m - sky).abs() < 0.25 * sky, "{level}: {m} vs {sky}");
        }
    }

    #[test]
    fn hammersley_points_fill_the_square() {
        let points: Vec<_> = (0..4).map(|i| hammersley(i, 4)).collect();
        assert_eq!(points[0], Vec2::ZERO);
        assert_eq!(points[1], Vec2::new(0.25, 0.5));
        assert_eq!(points[2], Vec2::new(0.5, 0.25));
        assert_eq!(points[3], Vec2::new(0.75, 0.75));
    }

    #[test]
    fn the_sampled_lobe_is_ggx() {
        // The mean cosine of GGX half vectors approaches 1 when smooth, lower when rough.
        let mean_cos = |alpha: f32| {
            (0..256)
                .map(|i| ggx_half_vector(hammersley(i, 256), alpha).1)
                .sum::<f32>()
                / 256.0
        };
        assert!(mean_cos(0.01) > 0.999);
        assert!(mean_cos(1.0) < 0.75);
        let h = ggx_half_vector(Vec2::new(0.3, 0.7), 0.5).0;
        assert!((h.length() - 1.0).abs() < 1e-5);
    }
}
