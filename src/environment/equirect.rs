//! Equirectangular sky images: the direction ↔ image mapping, bilinear sampling and a mip
//! pyramid for filtered lookups.
//!
//! The mapping is Blender's, so an image looks the same as in Blender's world shader: the
//! center column looks along +X, the column three quarters across along +Z, and the first row
//! straight up: `u = 0.5 + atan2(z, x) / 2π`, `v = acos(y) / π`.

use core::f32::consts::{PI, TAU};

use bevy::math::{Vec2, Vec3};
use rayon::prelude::*;

/// The `[0, 1]²` image coordinates of a unit direction.
pub fn direction_to_uv(direction: Vec3) -> Vec2 {
    Vec2::new(
        0.5 + direction.z.atan2(direction.x) / TAU,
        direction.y.clamp(-1.0, 1.0).acos() / PI,
    )
}

/// The unit direction at image coordinates `uv`; the inverse of [`direction_to_uv`].
pub fn uv_to_direction(uv: Vec2) -> Vec3 {
    let phi = (uv.x - 0.5) * TAU;
    let theta = uv.y * PI;
    let ring = theta.sin();
    Vec3::new(phi.cos() * ring, theta.cos(), phi.sin() * ring)
}

/// An equirectangular image of linear RGB radiance.
#[derive(Clone, Debug, PartialEq)]
pub struct Equirect {
    pub width: u32,
    pub height: u32,
    /// Row by row from the top.
    pub texels: Vec<Vec3>,
}

impl Equirect {
    pub fn new(width: u32, height: u32, texels: Vec<Vec3>) -> Self {
        assert!(width > 0 && height > 0, "an empty sky image");
        assert_eq!(texels.len(), (width * height) as usize, "texel count");
        Self {
            width,
            height,
            texels,
        }
    }

    /// An image of `radiance` in the direction of every texel center, for procedural skies.
    pub fn from_fn(width: u32, height: u32, radiance: impl Fn(Vec3) -> Vec3 + Sync) -> Self {
        let texels = (0..width * height)
            .into_par_iter()
            .map(|index| {
                let (x, y) = (index % width, index / width);
                let uv = Vec2::new(
                    (x as f32 + 0.5) / width as f32,
                    (y as f32 + 0.5) / height as f32,
                );
                radiance(uv_to_direction(uv))
            })
            .collect();
        Self::new(width, height, texels)
    }

    pub fn texel(&self, x: u32, y: u32) -> Vec3 {
        self.texels[(y * self.width + x) as usize]
    }

    /// The solid angle a texel of row `y` covers. The rows of an image sum to 4π.
    pub fn texel_solid_angle(&self, y: u32) -> f32 {
        let rows = self.height as f32;
        let top = (y as f32 / rows * PI).cos();
        let bottom = ((y + 1) as f32 / rows * PI).cos();
        TAU / self.width as f32 * (top - bottom)
    }

    /// The image bilinearly filtered at `uv`, wrapping around horizontally and clamped at the
    /// poles.
    pub fn sample(&self, uv: Vec2) -> Vec3 {
        let x = uv.x * self.width as f32 - 0.5;
        let y = (uv.y * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let column = |offset: f32| (x0 + offset).rem_euclid(self.width as f32) as u32;
        let (left, right) = (column(0.0), column(1.0));
        let top = y0 as u32;
        let bottom = (top + 1).min(self.height - 1);
        let row = |y: u32| self.texel(left, y).lerp(self.texel(right, y), fx);
        row(top).lerp(row(bottom), fy)
    }

    /// The image at half the size, each texel the solid-angle weighted mean of the 2×2 texels
    /// it covers. A side of 1 stays 1; the last row or column of an odd side is dropped.
    pub fn downsample(&self) -> Self {
        let width = (self.width / 2).max(1);
        let height = (self.height / 2).max(1);
        let texels = (0..width * height)
            .into_par_iter()
            .map(|index| {
                let (x, y) = (index % width, index / width);
                let mut sum = Vec3::ZERO;
                let mut weight = 0.0;
                for sy in [y * 2, y * 2 + 1] {
                    let sy = sy.min(self.height - 1);
                    let solid_angle = self.texel_solid_angle(sy);
                    for sx in [x * 2, x * 2 + 1] {
                        sum += self.texel(sx.min(self.width - 1), sy) * solid_angle;
                        weight += solid_angle;
                    }
                }
                sum / weight
            })
            .collect();
        Self::new(width, height, texels)
    }

    /// The image with every texel scaled down so no channel exceeds `max_radiance`, keeping its
    /// hue.
    pub fn clamped(&self, max_radiance: f32) -> Self {
        let texels = self
            .texels
            .par_iter()
            .map(|&texel| {
                let peak = texel.max_element();
                if peak > max_radiance {
                    texel * (max_radiance / peak)
                } else {
                    texel
                }
            })
            .collect();
        Self::new(self.width, self.height, texels)
    }
}

/// An image and its downsampled levels, down to 1×1.
#[derive(Clone, Debug, PartialEq)]
pub struct Pyramid {
    pub levels: Vec<Equirect>,
}

impl Pyramid {
    pub fn new(base: Equirect) -> Self {
        let mut levels = vec![base];
        while let Some(last) = levels.last().filter(|l| l.width > 1 || l.height > 1) {
            let next = last.downsample();
            levels.push(next);
        }
        Self { levels }
    }

    pub fn base(&self) -> &Equirect {
        &self.levels[0]
    }

    /// The mean solid angle of a base texel.
    pub fn base_texel_solid_angle(&self) -> f32 {
        4.0 * PI / (self.base().width * self.base().height) as f32
    }

    /// The radiance towards `direction`, blended between the two levels around `lod`.
    pub fn sample(&self, direction: Vec3, lod: f32) -> Vec3 {
        let uv = direction_to_uv(direction);
        let last = (self.levels.len() - 1) as f32;
        let lod = lod.clamp(0.0, last);
        let lower = lod.floor();
        let fine = self.levels[lower as usize].sample(uv);
        if lower == lod {
            return fine;
        }
        let coarse = self.levels[lower as usize + 1].sample(uv);
        fine.lerp(coarse, lod - lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mapping_is_blenders() {
        let center = uv_to_direction(Vec2::new(0.5, 0.5));
        assert!(center.abs_diff_eq(Vec3::X, 1e-6));
        let quarter = uv_to_direction(Vec2::new(0.75, 0.5));
        assert!(quarter.abs_diff_eq(Vec3::Z, 1e-6));
        assert!(uv_to_direction(Vec2::new(0.3, 0.0)).abs_diff_eq(Vec3::Y, 1e-6));
        let uv = direction_to_uv(Vec3::NEG_Z);
        assert!(uv.abs_diff_eq(Vec2::new(0.25, 0.5), 1e-6));
    }

    #[test]
    fn uv_round_trips() {
        for uv in [
            Vec2::new(0.1, 0.2),
            Vec2::new(0.6, 0.9),
            Vec2::new(0.95, 0.5),
        ] {
            let back = direction_to_uv(uv_to_direction(uv));
            assert!(back.abs_diff_eq(uv, 1e-5), "{uv} {back}");
        }
    }

    #[test]
    fn rows_cover_the_sphere() {
        let image = Equirect::from_fn(16, 8, |_| Vec3::ONE);
        let total: f32 = (0..8).map(|y| image.texel_solid_angle(y) * 16.0).sum();
        assert!((total - 4.0 * PI).abs() < 1e-4);
    }

    #[test]
    fn sampling_wraps_around_the_seam() {
        let mut image = Equirect::from_fn(4, 2, |_| Vec3::ZERO);
        image.texels[0] = Vec3::ONE;
        image.texels[4] = Vec3::ONE;
        // Halfway between the last column and the first.
        let seam = image.sample(Vec2::new(1.0, 0.5));
        assert!((seam.x - 0.5).abs() < 1e-6, "{seam}");
        assert!((image.sample(Vec2::new(0.125, 0.5)).x - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_pyramid_keeps_the_mean_radiance() {
        let image = Equirect::from_fn(64, 32, |d| Vec3::splat(d.y.max(0.0)));
        let pyramid = Pyramid::new(image);
        let sizes: Vec<_> = pyramid.levels.iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(sizes.first(), Some(&(64, 32)));
        assert_eq!(sizes.last(), Some(&(1, 1)));
        assert_eq!(sizes.len(), 7);
        // The upper hemisphere at y averages to 1/2, over the whole sphere 1/4.
        let top = pyramid.levels.last().unwrap().texels[0].x;
        assert!((top - 0.25).abs() < 0.01, "{top}");
        let blended = pyramid.sample(Vec3::Y, 0.5);
        assert!(blended.x > 0.9 && blended.x <= 1.0, "{blended}");
    }

    #[test]
    fn clamping_keeps_the_hue() {
        let image = Equirect::new(1, 1, vec![Vec3::new(100.0, 50.0, 10.0)]);
        let clamped = image.clamped(10.0);
        assert!(clamped.texels[0].abs_diff_eq(Vec3::new(10.0, 5.0, 1.0), 1e-5));
    }
}
