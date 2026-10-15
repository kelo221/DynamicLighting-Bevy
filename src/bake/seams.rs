//! Henry's `SeamsPaddingStep` and `DilateShadowBits`: fill texels no ray visited from their
//! visited neighbours, and grow each triangle's shadow bits into its 2 px padding.

use super::bits::{BitGrid, RegionBits};

/// Fills every unvisited texel of `bits` with the OR of its visited neighbours: the four direct
/// neighbours, and for each direction where that neighbour is unvisited, the texel two steps
/// away. `visited` covers the whole map.
///
/// Henry indexes neighbours as `y · size + x` and only checks the flat index, so `x - 1` at
/// `x = 0` reads the last texel of the previous row (and `x + 1` at the right edge the next
/// row). Neighbours outside the map are unvisited here.
pub fn pad_seams(bits: &mut RegionBits, visited: &BitGrid) {
    let rect = bits.rect;
    let visited_at = |x: i64, y: i64| visited.get_or_false(x, y);
    for y in rect.min_y..=rect.max_y {
        for x in rect.min_x..=rect.max_x {
            let (x, y) = (i64::from(x), i64::from(y));
            if visited_at(x, y) {
                continue;
            }
            let mut lit = false;
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x + dx, y + dy);
                if visited_at(nx, ny) {
                    lit |= bits.get(nx, ny);
                } else if visited_at(x + 2 * dx, y + 2 * dy) {
                    lit |= bits.get(x + 2 * dx, y + 2 * dy);
                }
            }
            // Only unvisited texels are written and only visited ones are read, so updating in
            // place gives the same result as Henry's parallel pass over a shared map.
            let (gx, gy) = (x as u32 - rect.min_x, y as u32 - rect.min_y);
            bits.grid.set(gx, gy, lit);
        }
    }
}

/// Grows the bits into the two outermost rings of the grid (the padding around a triangle's
/// texels): first the inner padding ring from the interior, then the outer ring from that.
pub fn dilate_shadow_bits(bits: &mut BitGrid) {
    let (width, height) = (bits.width(), bits.height());
    let mut copy = bits.clone();
    if height >= 2 {
        dilate_row(bits, &copy, 1);
    }
    if height >= 4 {
        dilate_row(bits, &copy, height - 2);
    }
    if width >= 2 {
        dilate_column(bits, &copy, 1);
    }
    if width >= 4 {
        dilate_column(bits, &copy, width - 2);
    }
    copy.clone_from(bits);
    if height >= 1 {
        dilate_row(bits, &copy, 0);
    }
    if height >= 3 {
        dilate_row(bits, &copy, height - 1);
    }
    if width >= 1 {
        dilate_column(bits, &copy, 0);
    }
    if width >= 3 {
        dilate_column(bits, &copy, width - 1);
    }
}

/// A texel becomes lit when it or any of its 8 neighbours in `copy` is lit.
fn dilated(copy: &BitGrid, x: u32, y: u32) -> bool {
    let (x, y) = (i64::from(x), i64::from(y));
    (-1..=1).any(|dy| (-1..=1).any(|dx| copy.get_or_false(x + dx, y + dy)))
}

fn dilate_row(bits: &mut BitGrid, copy: &BitGrid, y: u32) {
    for x in 0..bits.width() {
        let value = dilated(copy, x, y);
        bits.set(x, y, value);
    }
}

fn dilate_column(bits: &mut BitGrid, copy: &BitGrid, x: u32) {
    for y in 0..bits.height() {
        let value = dilated(copy, x, y);
        bits.set(x, y, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::bits::TexelRect;

    fn grid(width: u32, height: u32, set: &[(u32, u32)]) -> BitGrid {
        let mut grid = BitGrid::new(width, height);
        for &(x, y) in set {
            grid.set(x, y, true);
        }
        grid
    }

    fn region(size: u32, set: &[(u32, u32)]) -> RegionBits {
        RegionBits {
            rect: TexelRect {
                min_x: 0,
                min_y: 0,
                max_x: size - 1,
                max_y: size - 1,
            },
            grid: grid(size, size, set),
        }
    }

    #[test]
    fn unvisited_texels_take_visited_neighbours() {
        // Visited column x = 2 lit; texels left and right of it get filled, up to two away.
        let visited = grid(6, 1, &[(2, 0)]);
        let mut bits = region(6, &[(2, 0)]);
        pad_seams(&mut bits, &visited);
        let row: Vec<bool> = (0..6).map(|x| bits.grid.get(x, 0)).collect();
        assert_eq!(row, [true, true, true, true, true, false]);
    }

    #[test]
    fn padding_does_not_wrap_across_rows() {
        // A lit, visited texel at the end of row 0 must not leak into the start of row 1
        // (Henry's flat index makes (0, 1)'s left neighbour be (3, 0)).
        let visited = grid(4, 4, &[(3, 0)]);
        let mut bits = region(4, &[(3, 0)]);
        pad_seams(&mut bits, &visited);
        assert!(
            !bits.grid.get(0, 1),
            "x - 1 at x = 0 wrapped to the previous row"
        );
        assert!(bits.grid.get(2, 0) && bits.grid.get(3, 1));
        // And from the start of a row to the end of the previous one.
        let visited = grid(4, 4, &[(0, 1)]);
        let mut bits = region(4, &[(0, 1)]);
        pad_seams(&mut bits, &visited);
        assert!(
            !bits.grid.get(3, 0),
            "x + 1 at the right edge wrapped to the next row"
        );
    }

    #[test]
    fn visited_texels_are_never_changed() {
        let visited = grid(3, 3, &[(1, 1), (0, 0)]);
        let mut bits = region(3, &[(1, 1)]);
        pad_seams(&mut bits, &visited);
        assert!(!bits.grid.get(0, 0), "visited unlit texel stays unlit");
        assert!(bits.grid.get(1, 0) && bits.grid.get(0, 1));
    }

    #[test]
    fn dilation_fills_the_two_padding_rings() {
        // 7x7 grid: interior (2..=4) has one lit texel at its corner (2, 2).
        let mut bits = grid(7, 7, &[(2, 2)]);
        dilate_shadow_bits(&mut bits);
        // Inner ring (row/column 1) picks up neighbours of (2, 2).
        for (x, y) in [(1, 1), (2, 1), (3, 1), (1, 2), (1, 3)] {
            assert!(bits.get(x, y), "inner ring ({x}, {y})");
        }
        assert!(!bits.get(4, 1), "too far from the lit texel");
        // Outer ring grows from the dilated inner ring.
        for (x, y) in [(0, 0), (1, 0), (4, 0), (0, 4)] {
            assert!(bits.get(x, y), "outer ring ({x}, {y})");
        }
        assert!(!bits.get(5, 0) && !bits.get(6, 6));
        // The interior is untouched.
        assert!(!bits.get(3, 3));
    }

    #[test]
    fn dilation_handles_tiny_grids() {
        for (w, h) in [(1, 1), (1, 5), (5, 1), (2, 2), (3, 3)] {
            let mut bits = grid(w, h, &[(0, 0)]);
            dilate_shadow_bits(&mut bits);
            assert!(bits.get(0, 0));
        }
    }
}
