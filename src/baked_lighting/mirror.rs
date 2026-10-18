//! CPU mirror of the shadow filters in `shaders/triangles.wesl` (Henry's `shadow_sample_*` in
//! `DynamicTriangle`). Tests check the row reads against the texel-by-texel reads they replace,
//! for one-bit shadows, the soft levels of area lights and the sky's 8-bit levels.

use bevy::math::{IVec2, UVec2, Vec2};

/// `QUALITY_*` in `triangles.wesl`, the order of [`crate::shared::RuntimeQuality`].
pub const QUALITY_INTEGRATED: u32 = 0;
pub const QUALITY_LOW: u32 = 1;
pub const QUALITY_HIGH: u32 = 3;

/// One light's shadows on one triangle: grid texel `(x, y)` is texel `x + y · width` of the
/// words, `2^log_bits` bits each and `32 >> log_bits` per word, low bits first. Words past the
/// end read as zero, like a robust GPU buffer read.
#[derive(Clone, Copy, Debug)]
pub struct ShadowTexels<'a> {
    pub words: &'a [u32],
    /// The padded grid width, `bounds.z`.
    pub width: u32,
    /// The lightmap texel of grid texel (0, 0), `bounds.xy`.
    pub origin: UVec2,
    /// [`log_bits`] of the light's shadow bits per texel.
    pub log_bits: u32,
}

/// `shadow_log_bits`: 0, 1, 2 or 3 for 1, 2, 4 or 8 shadow bits per texel.
pub fn log_bits(bits_per_texel: u32) -> u32 {
    bits_per_texel.max(1).ilog2().min(3)
}

impl ShadowTexels<'_> {
    fn word(&self, index: u32) -> u32 {
        self.words.get(index as usize).copied().unwrap_or(0)
    }

    /// The largest level of a texel.
    pub fn top(&self) -> u32 {
        (1 << (1 << self.log_bits)) - 1
    }

    /// `shadow_row`: eight texels from grid texel `uv` rightward (four at 8 bits), texel `k` at
    /// bit `k · bits`.
    pub fn row(&self, uv: IVec2) -> u32 {
        let bits = 1 << self.log_bits;
        let span = 8.min(32 >> self.log_bits);
        let word_log = 5 - self.log_bits;
        let start = uv.y.max(0) * self.width as i32 + uv.x;
        let index = start.max(0) as u32;
        let word = index >> word_log;
        let shift = (index & ((1 << word_log) - 1)) << self.log_bits;
        let mut row = self.word(word) >> shift;
        if shift + span * bits > 32 {
            row |= self.word(word + 1) << (32 - shift);
        }
        if start < 0 {
            let before = ((-start).min(span as i32) as u32) << self.log_bits;
            let fill = (u32::MAX / self.top()) >> (32 - before);
            let moved = if before < 32 { row << before } else { 0 };
            row = moved | ((row & self.top()) * fill);
        }
        if bits < 4 {
            row & ((1 << (8 * bits)) - 1)
        } else {
            row
        }
    }

    /// One texel's level with the addressing of [`Self::row`], read on its own.
    pub fn texel(&self, uv: IVec2) -> f32 {
        let index = (uv.y.max(0) * self.width as i32 + uv.x).max(0) as u32;
        let word_log = 5 - self.log_bits;
        let shift = (index & ((1 << word_log) - 1)) << self.log_bits;
        ((self.word(index >> word_log) >> shift) & self.top()) as f32
    }

    /// `row_level`: the level of texel `x` of a row.
    fn level(&self, row: u32, x: u32) -> f32 {
        ((row >> (x << self.log_bits)) & self.top()) as f32
    }

    /// The first eight texels of `row_binomials`: one row read, or two at 8 bits.
    fn run(&self, uv: IVec2) -> [u32; 2] {
        let row = self.row(uv);
        if self.log_bits == 3 {
            [row, self.row(uv + IVec2::X * 4)]
        } else {
            [row, row >> (4 << self.log_bits)]
        }
    }

    fn binomial(&self, run: [u32; 2], x: u32) -> f32 {
        let t = |x: u32| self.level(run[(x / 4) as usize], x % 4);
        t(x) + t(x + 4) + 4.0 * (t(x + 1) + t(x + 3)) + 6.0 * t(x + 2)
    }

    fn corner(&self, pixel: Vec2) -> IVec2 {
        let texel = (pixel - 0.5).floor().max(Vec2::ZERO).as_uvec2();
        texel.wrapping_sub(self.origin).as_ivec2()
    }

    /// `baked_shadow` for an entry with shadow data, at lightmap texel coordinates `pixel`.
    pub fn shadow(&self, pixel: Vec2, quality: u32) -> f32 {
        let levels = match quality {
            QUALITY_INTEGRATED => self.integrated(pixel),
            QUALITY_LOW => self.box3(pixel),
            _ => self.bilinear(pixel, quality >= QUALITY_HIGH),
        };
        levels / self.top() as f32
    }

    fn integrated(&self, pixel: Vec2) -> f32 {
        let f = (pixel - 0.5).fract();
        let corner = self.corner(pixel);
        let top = self.row(corner);
        let bottom = self.row(corner + IVec2::Y);
        let left = Vec2::new(self.level(top, 0), self.level(bottom, 0));
        let right = Vec2::new(self.level(top, 1), self.level(bottom, 1));
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
        let mut sum = 0.0;
        for y in 0..3 {
            let row = self.row(uv + IVec2::new(0, y));
            sum += self.level(row, 0) + self.level(row, 1) + self.level(row, 2);
        }
        sum / 9.0
    }

    fn bilinear(&self, pixel: Vec2, gaussian: bool) -> f32 {
        let corner = self.corner(pixel);
        if !gaussian && self.log_bits == 0 {
            let mut block = 0;
            for y in 0..4 {
                let row = self.row(corner + IVec2::new(-1, y - 1));
                block |= (row & 0xF) << (4 * y);
            }
            let [tl, tr, bl, br] = window_counts(block).map(|count| count as f32);
            return blend_sums(tl, tr, bl, br, (pixel - 0.5).fract());
        }
        let mut map = [[0.0f32; 4]; 4];
        if gaussian {
            let mut across = [[0.0f32; 4]; 8];
            for (y, line) in across.iter_mut().enumerate() {
                let run = self.run(corner + IVec2::new(-3, y as i32 - 3));
                for (x, value) in line.iter_mut().enumerate() {
                    *value = self.binomial(run, x as u32);
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
                    *value = self.level(row, x as u32);
                }
            }
        }
        blend(&map, (pixel - 0.5).fract())
    }
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
    blend_sums(tl, tr, bl, br, f)
}

/// The four 3×3 sums blended bilinearly, over 9.
fn blend_sums(tl: f32, tr: f32, bl: f32, br: f32, f: Vec2) -> f32 {
    let (c0, c1) = (tl + (tr - tl) * f.x, bl + (br - bl) * f.x);
    (c0 + (c1 - c0) * f.y) / 9.0
}

/// `WINDOW_*` in `triangles.wesl`: the top-left, top-right, bottom-left and bottom-right 3×3
/// windows of a 4×4 block of one-bit texels, texel (x, y) at bit 4y + x.
pub const WINDOWS: [u32; 4] = [0x0777, 0x0EEE, 0x7770, 0xEEE0];

/// `shadow_bilinear_bits`: the texels set in each of the four windows of a block.
pub fn window_counts(block: u32) -> [u32; 4] {
    WINDOWS.map(|window| (block & window).count_ones())
}

#[cfg(test)]
#[path = "mirror_tests.rs"]
mod tests;
