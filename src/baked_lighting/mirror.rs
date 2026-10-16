//! CPU mirror of the shadow filters in `shaders/triangles.wesl` (Henry's `shadow_sample_*` in
//! `DynamicTriangle`). Tests check the row reads against the texel-by-texel reads they replace.

use bevy::math::{IVec2, UVec2, Vec2};

/// `QUALITY_*` in `triangles.wesl`, the order of [`crate::shared::RuntimeQuality`].
pub const QUALITY_INTEGRATED: u32 = 0;
pub const QUALITY_LOW: u32 = 1;
pub const QUALITY_HIGH: u32 = 3;

/// One light's shadow bits on one triangle: grid texel `(x, y)` is bit `x + y · width`.
/// Words past the end read as zero, like a robust GPU buffer read.
#[derive(Clone, Copy, Debug)]
pub struct ShadowBits<'a> {
    pub words: &'a [u32],
    /// The padded grid width, `bounds.z`.
    pub width: u32,
    /// The lightmap texel of grid texel (0, 0), `bounds.xy`.
    pub origin: UVec2,
}

impl ShadowBits<'_> {
    fn word(&self, index: u32) -> u32 {
        self.words.get(index as usize).copied().unwrap_or(0)
    }

    /// `shadow_row`: eight bits from grid texel `uv` rightward.
    pub fn row(&self, uv: IVec2) -> u32 {
        let start = uv.y.max(0) * self.width as i32 + uv.x;
        let index = start.max(0) as u32;
        let shift = index % 32;
        let mut bits = self.word(index / 32) >> shift;
        if shift > 24 {
            bits |= self.word(index / 32 + 1) << (32 - shift);
        }
        if start < 0 {
            let before = (-start).min(8) as u32;
            bits = (bits << before) | ((bits & 1) * ((1 << before) - 1));
        }
        bits & 0xff
    }

    /// One texel with the addressing of [`Self::row`], read on its own.
    pub fn texel(&self, uv: IVec2) -> f32 {
        let index = (uv.y.max(0) * self.width as i32 + uv.x).max(0) as u32;
        ((self.word(index / 32) >> (index % 32)) & 1) as f32
    }

    fn corner(&self, pixel: Vec2) -> IVec2 {
        let texel = (pixel - 0.5).floor().max(Vec2::ZERO).as_uvec2();
        texel.wrapping_sub(self.origin).as_ivec2()
    }

    /// `baked_shadow` for an entry with shadow data, at lightmap texel coordinates `pixel`.
    pub fn shadow(&self, pixel: Vec2, quality: u32) -> f32 {
        match quality {
            QUALITY_INTEGRATED => self.integrated(pixel),
            QUALITY_LOW => self.box3(pixel),
            _ => self.bilinear(pixel, quality >= QUALITY_HIGH),
        }
    }

    fn integrated(&self, pixel: Vec2) -> f32 {
        let f = (pixel - 0.5).fract();
        let corner = self.corner(pixel);
        let top = self.row(corner);
        let bottom = self.row(corner + IVec2::Y);
        let left = Vec2::new(bit(top, 0), bit(bottom, 0));
        let right = Vec2::new(bit(top, 1), bit(bottom, 1));
        let c = left.lerp(right, f.x);
        c.x + (c.y - c.x) * f.y
    }

    fn box3(&self, pixel: Vec2) -> f32 {
        let uv = pixel
            .max(Vec2::ZERO)
            .as_uvec2()
            .wrapping_sub(self.origin)
            .as_ivec2()
            - IVec2::ONE;
        let count: u32 = (0..3)
            .map(|y| (self.row(uv + IVec2::new(0, y)) & 7).count_ones())
            .sum();
        count as f32 / 9.0
    }

    fn bilinear(&self, pixel: Vec2, gaussian: bool) -> f32 {
        let corner = self.corner(pixel);
        let mut map = [[0.0f32; 4]; 4];
        if gaussian {
            let mut across = [[0.0f32; 4]; 8];
            for (y, line) in across.iter_mut().enumerate() {
                let row = self.row(corner + IVec2::new(-3, y as i32 - 3));
                for (x, value) in line.iter_mut().enumerate() {
                    *value = binomial(row, x as u32);
                }
            }
            for (y, line) in map.iter_mut().enumerate() {
                for (x, value) in line.iter_mut().enumerate() {
                    let side = across[y][x] + across[y + 4][x];
                    let near = across[y + 1][x] + across[y + 3][x];
                    *value = (side + 4.0 * near + 6.0 * across[y + 2][x]) / 256.0;
                }
            }
        } else {
            for (y, line) in map.iter_mut().enumerate() {
                let row = self.row(corner + IVec2::new(-1, y as i32 - 1));
                for (x, value) in line.iter_mut().enumerate() {
                    *value = bit(row, x as u32);
                }
            }
        }
        blend(&map, (pixel - 0.5).fract())
    }
}

fn bit(row: u32, x: u32) -> f32 {
    ((row >> x) & 1) as f32
}

fn binomial(row: u32, x: u32) -> f32 {
    bit(row, x)
        + bit(row, x + 4)
        + 4.0 * (bit(row, x + 1) + bit(row, x + 3))
        + 6.0 * bit(row, x + 2)
}

/// Four overlapping 3×3 sums of a 4×4 map, blended bilinearly, over 9.
fn blend(map: &[[f32; 4]; 4], f: Vec2) -> f32 {
    let middle = map[1][1] + map[1][2] + map[2][1] + map[2][2];
    let top = middle + map[0][1] + map[0][2];
    let tl = top + map[0][0] + map[1][0] + map[2][0];
    let tr = top + map[0][3] + map[1][3] + map[2][3];
    let bottom = middle + map[3][1] + map[3][2];
    let bl = bottom + map[1][0] + map[2][0] + map[3][0];
    let br = bottom + map[1][3] + map[2][3] + map[3][3];
    let (c0, c1) = (tl + (tr - tl) * f.x, bl + (br - bl) * f.x);
    (c0 + (c1 - c0) * f.y) / 9.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::EffectRng;

    /// The filters as they read one texel at a time before the row reads.
    fn texel_by_texel(bits: &ShadowBits, pixel: Vec2, quality: u32) -> f32 {
        let corner = bits.corner(pixel);
        let f = (pixel - 0.5).fract();
        match quality {
            QUALITY_INTEGRATED => {
                let t = |x, y| bits.texel(corner + IVec2::new(x, y));
                let c = Vec2::new(t(0, 0), t(0, 1)).lerp(Vec2::new(t(1, 0), t(1, 1)), f.x);
                c.x + (c.y - c.x) * f.y
            }
            QUALITY_LOW => {
                let uv = pixel
                    .max(Vec2::ZERO)
                    .as_uvec2()
                    .wrapping_sub(bits.origin)
                    .as_ivec2();
                let mut sum = 0.0;
                for y in -1..=1 {
                    for x in -1..=1 {
                        sum += bits.texel(uv + IVec2::new(x, y));
                    }
                }
                sum / 9.0
            }
            _ => {
                let weights = [1.0, 4.0, 6.0, 4.0, 1.0];
                let mut map = [[0.0f32; 4]; 4];
                for (y, line) in map.iter_mut().enumerate() {
                    for (x, value) in line.iter_mut().enumerate() {
                        let texel = corner + IVec2::new(x as i32 - 1, y as i32 - 1);
                        if quality < QUALITY_HIGH {
                            *value = bits.texel(texel);
                            continue;
                        }
                        let mut sum = 0.0;
                        for (j, wy) in weights.iter().enumerate() {
                            for (i, wx) in weights.iter().enumerate() {
                                let tap = texel + IVec2::new(i as i32 - 2, j as i32 - 2);
                                sum += wx * wy * bits.texel(tap);
                            }
                        }
                        *value = sum / 256.0;
                    }
                }
                blend(&map, f)
            }
        }
    }

    #[test]
    fn row_reads_match_texel_reads_everywhere() {
        let mut rng = EffectRng::new(7, 11);
        for width in [5u32, 7, 13, 31, 32, 33, 40] {
            let height = 9u32;
            let words: Vec<u32> = (0..(width * height).div_ceil(32) + 1)
                .map(|_| rng.next_u32() & rng.next_u32())
                .collect();
            let bits = ShadowBits {
                words: &words,
                width,
                origin: UVec2::new(3, 5),
            };
            // Every footprint over the triangle's texels and the padding around them.
            for i in 0..2000 {
                let pixel = Vec2::new(
                    3.0 + (i as f32 * 0.618).fract() * width as f32,
                    5.0 + (i as f32 * 0.414).fract() * height as f32,
                );
                for quality in 0..4 {
                    let rows = bits.shadow(pixel, quality);
                    let texels = texel_by_texel(&bits, pixel, quality);
                    assert_eq!(rows, texels, "{width} {pixel} {quality}");
                }
            }
        }
    }

    #[test]
    fn reads_above_and_before_the_grid_repeat_its_first_texels() {
        let words = [0b1011_0001u32, 0];
        let bits = ShadowBits {
            words: &words,
            width: 8,
            origin: UVec2::ZERO,
        };
        assert_eq!(bits.row(IVec2::new(0, -2)), bits.row(IVec2::ZERO));
        // Two texels before the first: copies of texel 0, then texels 0 to 5.
        assert_eq!(bits.row(IVec2::new(-2, 0)), 0b1100_0111);
        // A run past the right side continues in the next row, as Henry's index does.
        assert_eq!(bits.row(IVec2::new(6, 0)) & 0b11, 0b10);
    }

    #[test]
    fn runs_across_a_word_boundary_join_both_words() {
        let words = [0xf000_0000u32, 0x0000_0005];
        let bits = ShadowBits {
            words: &words,
            width: 64,
            origin: UVec2::ZERO,
        };
        assert_eq!(bits.row(IVec2::new(28, 0)), 0b0101_1111);
        assert_eq!(bits.row(IVec2::new(24, 0)), 0xf0);
    }
}
