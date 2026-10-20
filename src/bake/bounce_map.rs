//! One mesh's bounce texture for one light: Henry's full-size float map, dilation
//! (`DilateBounceTexture`), separable Gaussian blur (`GaussianBlur.ApplyGaussianBlur`, after
//! Daniel Ilett's SmoShaders) and the per-triangle crops (`BuildBounceTextures`).
//!
//! Only a window of the map is stored: every non-zero gathered texel lies inside the light's
//! triangle crops, so a window grown by the dilation reach (1) plus the blur radius (3) gives
//! exactly the values a full map would. Reads outside the window are 0; the blur clamps at the
//! map edges like Henry's.

use bevy::math::Vec3;

use super::bits::TexelRect;

/// Henry's blur: 7 taps, sigma 5.
pub const BLUR_KERNEL_SIZE: usize = 7;
pub const BLUR_SIGMA: f32 = 5.0;
/// How far beyond the triangle crops the window reaches: dilation (1) plus blur radius (3).
pub const WINDOW_MARGIN: u32 = 1 + (BLUR_KERNEL_SIZE as u32) / 2;

/// A window of a `size`² bounce texture, red, green and blue (all equal for white bounce).
#[derive(Clone, Debug, PartialEq)]
pub struct BounceMap {
    size: u32,
    rect: TexelRect,
    values: Vec<Vec3>,
}

impl BounceMap {
    /// A zeroed window covering `crops` grown by [`WINDOW_MARGIN`], clamped to the map.
    pub fn new(size: u32, crops: TexelRect) -> Self {
        let last = size - 1;
        let rect = TexelRect {
            min_x: crops.min_x.saturating_sub(WINDOW_MARGIN),
            min_y: crops.min_y.saturating_sub(WINDOW_MARGIN),
            max_x: (crops.max_x + WINDOW_MARGIN).min(last),
            max_y: (crops.max_y + WINDOW_MARGIN).min(last),
        };
        Self {
            size,
            rect,
            values: vec![Vec3::ZERO; (rect.width() * rect.height()) as usize],
        }
    }

    fn index(&self, x: u32, y: u32) -> usize {
        ((y - self.rect.min_y) * self.rect.width() + (x - self.rect.min_x)) as usize
    }

    /// The value at map texel `(x, y)`; 0 outside the map or the window.
    pub fn get(&self, x: i64, y: i64) -> Vec3 {
        if self.rect.contains(x, y) {
            self.values[self.index(x as u32, y as u32)]
        } else {
            Vec3::ZERO
        }
    }

    /// Writes a gathered texel, which must lie inside the window.
    pub fn set(&mut self, x: u32, y: u32, value: Vec3) {
        let index = self.index(x, y);
        self.values[index] = value;
    }

    /// `DilateBounceTexture`: every texel takes the first positive value among itself and its
    /// eight neighbours, in Henry's order. A value with any positive channel counts, and its
    /// channels move together.
    pub fn dilate(&mut self) {
        const ORDER: [(i64, i64); 9] = [
            (0, 0),
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ];
        let values = self.map_window(|map, x, y| {
            ORDER
                .iter()
                .map(|&(dx, dy)| map.get(x + dx, y + dy))
                .find(|&value| value.max_element() > 0.0)
                .unwrap_or(Vec3::ZERO)
        });
        self.values = values;
    }

    /// Henry's separable Gaussian blur, horizontal then vertical, with sample coordinates
    /// clamped to the map. Henry's passes run in place (source and destination alias), which
    /// smears values along each row and column; this blurs from a copy instead.
    pub fn blur(&mut self) {
        let weights = gaussian_weights(BLUR_KERNEL_SIZE, BLUR_SIGMA);
        let weight_sum: f32 = weights.iter().sum();
        let half = (BLUR_KERNEL_SIZE / 2) as i64;
        let last = i64::from(self.size) - 1;
        for vertical in [false, true] {
            let values = self.map_window(|map, x, y| {
                let mut sum = Vec3::ZERO;
                for (k, &weight) in (-half..=half).zip(&weights) {
                    let sample = if vertical {
                        map.get(x, (y + k).clamp(0, last))
                    } else {
                        map.get((x + k).clamp(0, last), y)
                    };
                    sum += sample * weight;
                }
                sum / weight_sum
            });
            self.values = values;
        }
    }

    fn map_window(&self, f: impl Fn(&Self, i64, i64) -> Vec3) -> Vec<Vec3> {
        let r = self.rect;
        (r.min_y..=r.max_y)
            .flat_map(|y| (r.min_x..=r.max_x).map(move |x| (x, y)))
            .map(|(x, y)| f(self, i64::from(x), i64::from(y)))
            .collect()
    }

    /// `BuildBounceTextures`: the crop copied into a grid with 2 px of zero padding on every
    /// side (the shadow bits' padded layout), or `None` when nothing in the crop is lit.
    pub fn crop(&self, crop: TexelRect) -> Option<Vec<Vec3>> {
        let width = crop.width() + 4;
        let mut grid = vec![Vec3::ZERO; (width * (crop.height() + 4)) as usize];
        let mut lit = false;
        for y in crop.min_y..=crop.max_y {
            for x in crop.min_x..=crop.max_x {
                let value = self.get(i64::from(x), i64::from(y));
                lit |= value.max_element() > 0.0;
                grid[((y - crop.min_y + 2) * width + (x - crop.min_x + 2)) as usize] = value;
            }
        }
        lit.then_some(grid)
    }
}

/// Normalized Gaussian weights for `-size/2 ..= size/2`.
pub fn gaussian_weights(size: usize, sigma: f32) -> Vec<f32> {
    let half = (size / 2) as i32;
    let sigma2 = sigma * sigma;
    let weights: Vec<f32> = (-half..=half)
        .map(|i| {
            (1.0 / (2.0 * std::f32::consts::PI * sigma2).sqrt())
                * (-((i * i) as f32) / (2.0 * sigma2)).exp()
        })
        .collect();
    let sum: f32 = weights.iter().sum();
    weights.into_iter().map(|w| w / sum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(min_x: u32, min_y: u32, max_x: u32, max_y: u32) -> TexelRect {
        TexelRect {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    /// The same filters on a plain full-size map, as Henry runs them (out of place).
    fn full_map(size: u32, texels: &[(u32, u32, f32)]) -> Vec<f32> {
        let s = size as i64;
        let mut map = vec![0.0f32; (size * size) as usize];
        for &(x, y, v) in texels {
            map[(y * size + x) as usize] = v;
        }
        let at = |m: &[f32], x: i64, y: i64| {
            if x < 0 || y < 0 || x >= s || y >= s {
                0.0
            } else {
                m[(y * s + x) as usize]
            }
        };
        let order = [
            (0, 0),
            (-1, -1),
            (0, -1),
            (1, -1),
            (-1, 0),
            (1, 0),
            (-1, 1),
            (0, 1),
            (1, 1),
        ];
        let mut dilated = map.clone();
        for y in 0..s {
            for x in 0..s {
                dilated[(y * s + x) as usize] = order
                    .iter()
                    .map(|&(dx, dy)| at(&map, x + dx, y + dy))
                    .find(|&v| v > 0.0)
                    .unwrap_or(0.0);
            }
        }
        let w = gaussian_weights(7, 5.0);
        let mut current = dilated;
        for vertical in [false, true] {
            let mut next = current.clone();
            for y in 0..s {
                for x in 0..s {
                    let mut sum = 0.0;
                    for k in -3..=3i64 {
                        let (sx, sy) = if vertical {
                            (x, (y + k).clamp(0, s - 1))
                        } else {
                            ((x + k).clamp(0, s - 1), y)
                        };
                        sum += at(&current, sx, sy) * w[(k + 3) as usize];
                    }
                    next[(y * s + x) as usize] = sum / w.iter().sum::<f32>();
                }
            }
            current = next;
        }
        current
    }

    #[test]
    fn windowed_filters_match_a_full_map() {
        let size = 24;
        let texels = [
            (1, 1, 0.5),
            (2, 1, 0.25),
            (12, 13, 1.0),
            (13, 13, 0.75),
            (0, 3, 0.1),
        ];
        let crops = rect(0, 0, 14, 14);
        let mut map = BounceMap::new(size, crops);
        for &(x, y, v) in &texels {
            map.set(x, y, Vec3::new(v, v * 0.5, v * 0.25));
        }
        map.dilate();
        map.blur();
        let full = full_map(size, &texels);
        for y in 0..=14u32 {
            for x in 0..=14u32 {
                let expected = full[(y * size + x) as usize];
                let got = map.get(i64::from(x), i64::from(y));
                let expected = Vec3::new(expected, expected * 0.5, expected * 0.25);
                assert!(
                    expected.abs_diff_eq(got, 1e-6),
                    "({x}, {y}): {expected} vs {got}"
                );
            }
        }
    }

    #[test]
    fn weights_are_normalized_and_symmetric() {
        let w = gaussian_weights(7, 5.0);
        assert_eq!(w.len(), 7);
        assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        assert_eq!(w[0], w[6]);
        assert!(w[3] > w[2] && w[2] > w[1]);
    }

    #[test]
    fn crops_are_padded_and_dropped_when_dark() {
        let mut map = BounceMap::new(16, rect(4, 4, 6, 5));
        map.set(5, 4, Vec3::new(0.0, 0.5, 0.0));
        let grid = map.crop(rect(4, 4, 6, 5)).unwrap();
        assert_eq!(grid.len(), 7 * 6);
        assert_eq!(
            grid[2 * 7 + 3],
            Vec3::new(0.0, 0.5, 0.0),
            "texel (5, 4) lands at (3, 2)"
        );
        assert_eq!(
            grid.iter().filter(|v| v.max_element() > 0.0).count(),
            1,
            "padding stays zero"
        );
        assert_eq!(map.crop(rect(10, 10, 11, 11)), None);
    }
}
