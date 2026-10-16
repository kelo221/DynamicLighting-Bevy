//! Bit grids (one bit per texel, LSB-first, bit `x + y · width`) and texel rectangles.

use std::sync::atomic::{AtomicU32, Ordering};

/// An inclusive rectangle of texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TexelRect {
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
}

impl TexelRect {
    pub fn width(&self) -> u32 {
        self.max_x - self.min_x + 1
    }

    pub fn height(&self) -> u32 {
        self.max_y - self.min_y + 1
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    pub fn contains(&self, x: i64, y: i64) -> bool {
        x >= i64::from(self.min_x)
            && y >= i64::from(self.min_y)
            && x <= i64::from(self.max_x)
            && y <= i64::from(self.max_y)
    }
}

/// A 2D grid of bits packed into `u32` words exactly as Henry's `BitArray2`: bit `x + y · width`
/// is word `i / 32`, bit `i % 32`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitGrid {
    width: u32,
    height: u32,
    words: Vec<u32>,
}

impl BitGrid {
    pub fn new(width: u32, height: u32) -> Self {
        let bits = width as usize * height as usize;
        Self {
            width,
            height,
            words: vec![0; bits.div_ceil(32)],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn get(&self, x: u32, y: u32) -> bool {
        debug_assert!(x < self.width && y < self.height);
        let index = (x + y * self.width) as usize;
        self.words[index / 32] & (1 << (index % 32)) != 0
    }

    /// [`Self::get`] that treats coordinates outside the grid as unset.
    pub fn get_or_false(&self, x: i64, y: i64) -> bool {
        x >= 0
            && y >= 0
            && x < i64::from(self.width)
            && y < i64::from(self.height)
            && self.get(x as u32, y as u32)
    }

    pub fn set(&mut self, x: u32, y: u32, value: bool) {
        debug_assert!(x < self.width && y < self.height);
        let index = (x + y * self.width) as usize;
        let bit = 1 << (index % 32);
        if value {
            self.words[index / 32] |= bit;
        } else {
            self.words[index / 32] &= !bit;
        }
    }

    #[cfg(test)]
    pub fn words(&self) -> &[u32] {
        &self.words
    }

    pub fn into_words(self) -> Vec<u32> {
        self.words
    }
}

/// A [`BitGrid`] that many threads can set bits in at once. OR is commutative, so the result
/// doesn't depend on thread scheduling.
pub struct AtomicBitGrid {
    width: u32,
    height: u32,
    words: Vec<AtomicU32>,
}

impl AtomicBitGrid {
    pub fn new(width: u32, height: u32) -> Self {
        let bits = width as usize * height as usize;
        Self {
            width,
            height,
            words: (0..bits.div_ceil(32)).map(|_| AtomicU32::new(0)).collect(),
        }
    }

    pub fn set(&self, x: u32, y: u32) {
        debug_assert!(x < self.width && y < self.height);
        let index = (x + y * self.width) as usize;
        self.words[index / 32].fetch_or(1 << (index % 32), Ordering::Relaxed);
    }

    pub fn into_grid(self) -> BitGrid {
        BitGrid {
            width: self.width,
            height: self.height,
            words: self.words.into_iter().map(AtomicU32::into_inner).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_are_lsb_first_row_major() {
        let mut grid = BitGrid::new(5, 7);
        assert_eq!(grid.words().len(), 2); // 35 bits
        grid.set(0, 0, true);
        grid.set(1, 0, true);
        grid.set(2, 6, true); // bit 32
        assert_eq!(grid.words(), &[0b11, 1]);
        assert!(grid.get(2, 6) && !grid.get(3, 6));
        assert!(!grid.get_or_false(-1, 0) && !grid.get_or_false(0, 7));
        grid.set(0, 0, false);
        assert_eq!(grid.words()[0], 0b10);
    }

    #[test]
    fn atomic_grid_matches_plain_grid() {
        let atomic = AtomicBitGrid::new(9, 9);
        let mut plain = BitGrid::new(9, 9);
        for (x, y) in [(0, 0), (8, 8), (3, 4), (3, 4)] {
            atomic.set(x, y);
            plain.set(x, y, true);
        }
        assert_eq!(atomic.into_grid(), plain);
    }
}
