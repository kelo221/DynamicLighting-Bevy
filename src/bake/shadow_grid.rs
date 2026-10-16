//! A light's shadow values over texels: one bit for point-like lights, or a few bits of visibility
//! for the soft shadows of area lights. A bit is the level 0 or 1, so seam padding, dilation and
//! cropping treat both alike and combine texels with `max` (for bits, OR).
//!
//! Packed words hold `floor(32 / bits)` texels, texel `j` of a word at bit `j · bits`, like bounce
//! texels. One bit per texel is exactly the [`BitGrid`] layout.

use std::sync::atomic::{AtomicU8, Ordering};

use super::bits::{AtomicBitGrid, BitGrid, TexelRect};

/// The level for a lit fraction of a light, at `bits` per texel.
pub fn level(fraction: f32, bits: u32) -> u8 {
    let max = ((1u32 << bits) - 1) as f32;
    (fraction.clamp(0.0, 1.0) * max).round() as u8
}

/// Texel levels from 0 (shadowed) to `2^bits − 1` (lit).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShadowGrid {
    Bits(BitGrid),
    Levels {
        width: u32,
        height: u32,
        bits: u32,
        levels: Vec<u8>,
    },
}

impl ShadowGrid {
    /// An all-shadowed grid with `bits` per texel, 1 to 8.
    pub fn new(width: u32, height: u32, bits: u32) -> Self {
        if bits <= 1 {
            return Self::Bits(BitGrid::new(width, height));
        }
        Self::Levels {
            width,
            height,
            bits: bits.min(8),
            levels: vec![0; width as usize * height as usize],
        }
    }

    pub fn bits_per_texel(&self) -> u32 {
        match self {
            Self::Bits(_) => 1,
            Self::Levels { bits, .. } => *bits,
        }
    }

    pub fn width(&self) -> u32 {
        match self {
            Self::Bits(grid) => grid.width(),
            Self::Levels { width, .. } => *width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Self::Bits(grid) => grid.height(),
            Self::Levels { height, .. } => *height,
        }
    }

    pub fn get(&self, x: u32, y: u32) -> u8 {
        match self {
            Self::Bits(grid) => u8::from(grid.get(x, y)),
            Self::Levels { width, levels, .. } => levels[(x + y * width) as usize],
        }
    }

    /// [`Self::get`] that reads coordinates outside the grid as 0.
    pub fn get_or_zero(&self, x: i64, y: i64) -> u8 {
        let inside =
            x >= 0 && y >= 0 && x < i64::from(self.width()) && y < i64::from(self.height());
        if inside {
            self.get(x as u32, y as u32)
        } else {
            0
        }
    }

    pub fn set(&mut self, x: u32, y: u32, level: u8) {
        match self {
            Self::Bits(grid) => grid.set(x, y, level != 0),
            Self::Levels { width, levels, .. } => levels[(x + y * *width) as usize] = level,
        }
    }

    /// The packed words.
    pub fn into_words(self) -> Vec<u32> {
        match self {
            Self::Bits(grid) => grid.into_words(),
            Self::Levels { bits, levels, .. } => {
                let per_word = (32 / bits) as usize;
                levels
                    .chunks(per_word)
                    .map(|chunk| {
                        chunk.iter().enumerate().fold(0, |word, (j, &level)| {
                            word | u32::from(level) << (j as u32 * bits)
                        })
                    })
                    .collect()
            }
        }
    }
}

/// A [`ShadowGrid`] many threads can raise texels in at once. `max` is commutative, so the result
/// doesn't depend on thread scheduling.
pub enum AtomicShadowGrid {
    Bits(AtomicBitGrid),
    Levels {
        width: u32,
        height: u32,
        bits: u32,
        levels: Vec<AtomicU8>,
    },
}

impl AtomicShadowGrid {
    pub fn new(width: u32, height: u32, bits: u32) -> Self {
        if bits <= 1 {
            return Self::Bits(AtomicBitGrid::new(width, height));
        }
        let count = width as usize * height as usize;
        Self::Levels {
            width,
            height,
            bits: bits.min(8),
            levels: (0..count).map(|_| AtomicU8::new(0)).collect(),
        }
    }

    /// Raises texel `(x, y)` to at least `level`.
    pub fn raise(&self, x: u32, y: u32, level: u8) {
        match self {
            Self::Bits(grid) if level != 0 => grid.set(x, y),
            Self::Bits(_) => {}
            Self::Levels { width, levels, .. } => {
                levels[(x + y * width) as usize].fetch_max(level, Ordering::Relaxed);
            }
        }
    }

    pub fn into_grid(self) -> ShadowGrid {
        match self {
            Self::Bits(grid) => ShadowGrid::Bits(grid.into_grid()),
            Self::Levels {
                width,
                height,
                bits,
                levels,
            } => ShadowGrid::Levels {
                width,
                height,
                bits,
                levels: levels.into_iter().map(AtomicU8::into_inner).collect(),
            },
        }
    }
}

/// Shadow values covering only `rect` of a larger map; reads outside it are 0.
pub struct ShadowRegion {
    pub rect: TexelRect,
    pub grid: ShadowGrid,
}

impl ShadowRegion {
    /// Level at lightmap texel `(x, y)`.
    pub fn get(&self, x: i64, y: i64) -> u8 {
        self.grid.get_or_zero(
            x - i64::from(self.rect.min_x),
            y - i64::from(self.rect.min_y),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_pack_eight_per_word_low_bits_first() {
        let mut grid = ShadowGrid::new(3, 3, 4);
        grid.set(0, 0, 15);
        grid.set(1, 0, 1);
        grid.set(2, 2, 7); // texel 8: first of the second word
        assert_eq!((grid.get(1, 0), grid.get_or_zero(3, 0)), (1, 0));
        assert_eq!(grid.into_words(), [0x1F, 7]);
    }

    #[test]
    fn one_bit_grids_keep_the_bit_layout() {
        let mut grid = ShadowGrid::new(5, 7, 1);
        let mut bits = BitGrid::new(5, 7);
        for (x, y) in [(0, 0), (4, 1), (2, 6)] {
            grid.set(x, y, 1);
            bits.set(x, y, true);
        }
        assert_eq!(grid.bits_per_texel(), 1);
        assert_eq!(grid.into_words(), bits.into_words());
    }

    #[test]
    fn atomic_raises_keep_the_largest_level() {
        for bits in [1, 4] {
            let atomic = AtomicShadowGrid::new(4, 4, bits);
            let mut plain = ShadowGrid::new(4, 4, bits);
            let top = level(1.0, bits);
            for (x, y, value) in [(1, 1, top), (1, 1, 0), (2, 3, 1), (2, 3, top), (0, 0, 0)] {
                atomic.raise(x, y, value);
                let raised = plain.get(x, y).max(value);
                plain.set(x, y, raised);
            }
            assert_eq!(atomic.into_grid(), plain);
        }
    }

    #[test]
    fn fractions_round_to_the_nearest_level() {
        assert_eq!(level(0.0, 4), 0);
        assert_eq!(level(0.5, 4), 8);
        assert_eq!(level(1.2, 4), 15);
        assert_eq!(level(0.49, 1), 0);
        assert_eq!(level(0.51, 1), 1);
    }

    #[test]
    fn regions_offset_reads() {
        let rect = TexelRect {
            min_x: 10,
            min_y: 20,
            max_x: 12,
            max_y: 21,
        };
        let mut grid = ShadowGrid::new(rect.width(), rect.height(), 4);
        grid.set(2, 1, 9);
        let region = ShadowRegion { rect, grid };
        assert_eq!(region.get(12, 21), 9);
        assert_eq!((region.get(13, 21), region.get(9, 20)), (0, 0));
        assert!(rect.contains(10, 21) && !rect.contains(10, 22));
    }
}
