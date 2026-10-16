//! Henry's `SeamsPaddingStep` and `DilateShadowBits`: fill texels no ray visited from their
//! visited neighbours, and grow each triangle's shadows into its 2 px padding. Henry's OR of bits
//! is the `max` of levels here, so soft area-light shadows pad the same way.

use super::bits::BitGrid;
use super::shadow_grid::{ShadowGrid, ShadowRegion};

/// Fills every unvisited texel of `region` with the largest level of its visited neighbours: the
/// four direct neighbours, and for each direction where that neighbour is unvisited, the texel
/// two steps away. `visited` covers the whole map.
///
/// Henry indexes neighbours as `y · size + x` and only checks the flat index, so `x - 1` at
/// `x = 0` reads the last texel of the previous row (and `x + 1` at the right edge the next
/// row). Neighbours outside the map are unvisited here.
pub fn pad_seams(region: &mut ShadowRegion, visited: &BitGrid) {
    let rect = region.rect;
    let visited_at = |x: i64, y: i64| visited.get_or_false(x, y);
    for y in rect.min_y..=rect.max_y {
        for x in rect.min_x..=rect.max_x {
            let (x, y) = (i64::from(x), i64::from(y));
            if visited_at(x, y) {
                continue;
            }
            let mut level = 0;
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x + dx, y + dy);
                if visited_at(nx, ny) {
                    level = level.max(region.get(nx, ny));
                } else if visited_at(x + 2 * dx, y + 2 * dy) {
                    level = level.max(region.get(x + 2 * dx, y + 2 * dy));
                }
            }
            // Only unvisited texels are written and only visited ones are read, so updating in
            // place gives the same result as Henry's parallel pass over a shared map.
            let (gx, gy) = (x as u32 - rect.min_x, y as u32 - rect.min_y);
            region.grid.set(gx, gy, level);
        }
    }
}

/// Grows the shadows into the two outermost rings of the grid (the padding around a triangle's
/// texels): first the inner padding ring from the interior, then the outer ring from that.
pub fn dilate_shadows(grid: &mut ShadowGrid) {
    let (width, height) = (grid.width(), grid.height());
    let mut copy = grid.clone();
    if height >= 2 {
        dilate_row(grid, &copy, 1);
    }
    if height >= 4 {
        dilate_row(grid, &copy, height - 2);
    }
    if width >= 2 {
        dilate_column(grid, &copy, 1);
    }
    if width >= 4 {
        dilate_column(grid, &copy, width - 2);
    }
    copy.clone_from(grid);
    if height >= 1 {
        dilate_row(grid, &copy, 0);
    }
    if height >= 3 {
        dilate_row(grid, &copy, height - 1);
    }
    if width >= 1 {
        dilate_column(grid, &copy, 0);
    }
    if width >= 3 {
        dilate_column(grid, &copy, width - 1);
    }
}

/// The largest level of a texel and its 8 neighbours in `copy`.
fn dilated(copy: &ShadowGrid, x: u32, y: u32) -> u8 {
    let (x, y) = (i64::from(x), i64::from(y));
    (-1..=1)
        .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| copy.get_or_zero(x + dx, y + dy))
        .max()
        .unwrap_or(0)
}

fn dilate_row(grid: &mut ShadowGrid, copy: &ShadowGrid, y: u32) {
    for x in 0..grid.width() {
        let value = dilated(copy, x, y);
        grid.set(x, y, value);
    }
}

fn dilate_column(grid: &mut ShadowGrid, copy: &ShadowGrid, x: u32) {
    for y in 0..grid.height() {
        let value = dilated(copy, x, y);
        grid.set(x, y, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::bits::TexelRect;

    fn visited(width: u32, height: u32, set: &[(u32, u32)]) -> BitGrid {
        let mut grid = BitGrid::new(width, height);
        for &(x, y) in set {
            grid.set(x, y, true);
        }
        grid
    }

    fn grid(width: u32, height: u32, set: &[(u32, u32)]) -> ShadowGrid {
        ShadowGrid::Bits(visited(width, height, set))
    }

    fn rect(width: u32, height: u32) -> TexelRect {
        TexelRect {
            min_x: 0,
            min_y: 0,
            max_x: width - 1,
            max_y: height - 1,
        }
    }

    fn region(size: u32, set: &[(u32, u32)]) -> ShadowRegion {
        ShadowRegion {
            rect: rect(size, size),
            grid: grid(size, size, set),
        }
    }

    fn lit(grid: &ShadowGrid, x: u32, y: u32) -> bool {
        grid.get(x, y) != 0
    }

    #[test]
    fn unvisited_texels_take_visited_neighbours() {
        // Visited column x = 2 lit; texels left and right of it get filled, up to two away.
        let seen = visited(6, 1, &[(2, 0)]);
        let mut bits = region(6, &[(2, 0)]);
        pad_seams(&mut bits, &seen);
        let row: Vec<bool> = (0..6).map(|x| lit(&bits.grid, x, 0)).collect();
        assert_eq!(row, [true, true, true, true, true, false]);
    }

    #[test]
    fn padding_does_not_wrap_across_rows() {
        // A lit, visited texel at the end of row 0 must not leak into the start of row 1
        // (Henry's flat index makes (0, 1)'s left neighbour be (3, 0)).
        let seen = visited(4, 4, &[(3, 0)]);
        let mut bits = region(4, &[(3, 0)]);
        pad_seams(&mut bits, &seen);
        assert!(
            !lit(&bits.grid, 0, 1),
            "x - 1 at x = 0 wrapped to the previous row"
        );
        assert!(lit(&bits.grid, 2, 0) && lit(&bits.grid, 3, 1));
        // And from the start of a row to the end of the previous one.
        let seen = visited(4, 4, &[(0, 1)]);
        let mut bits = region(4, &[(0, 1)]);
        pad_seams(&mut bits, &seen);
        assert!(
            !lit(&bits.grid, 3, 0),
            "x + 1 at the right edge wrapped to the next row"
        );
    }

    #[test]
    fn visited_texels_are_never_changed() {
        let seen = visited(3, 3, &[(1, 1), (0, 0)]);
        let mut bits = region(3, &[(1, 1)]);
        pad_seams(&mut bits, &seen);
        assert!(!lit(&bits.grid, 0, 0), "visited unlit texel stays unlit");
        assert!(lit(&bits.grid, 1, 0) && lit(&bits.grid, 0, 1));
    }

    #[test]
    fn seams_take_the_brightest_soft_neighbour() {
        let seen = visited(3, 1, &[(0, 0), (2, 0)]);
        let mut levels = ShadowGrid::new(3, 1, 4);
        levels.set(0, 0, 4);
        levels.set(2, 0, 11);
        let mut region = ShadowRegion {
            rect: rect(3, 1),
            grid: levels,
        };
        pad_seams(&mut region, &seen);
        let row = (region.get(0, 0), region.get(1, 0), region.get(2, 0));
        assert_eq!(row, (4, 11, 11));
    }

    #[test]
    fn dilation_fills_the_two_padding_rings() {
        // 7x7 grid: interior (2..=4) has one lit texel at its corner (2, 2).
        let mut bits = grid(7, 7, &[(2, 2)]);
        dilate_shadows(&mut bits);
        // Inner ring (row/column 1) picks up neighbours of (2, 2).
        for (x, y) in [(1, 1), (2, 1), (3, 1), (1, 2), (1, 3)] {
            assert!(lit(&bits, x, y), "inner ring ({x}, {y})");
        }
        assert!(!lit(&bits, 4, 1), "too far from the lit texel");
        // Outer ring grows from the dilated inner ring.
        for (x, y) in [(0, 0), (1, 0), (4, 0), (0, 4)] {
            assert!(lit(&bits, x, y), "outer ring ({x}, {y})");
        }
        assert!(!lit(&bits, 5, 0) && !lit(&bits, 6, 6));
        // The interior is untouched.
        assert!(!lit(&bits, 3, 3));
    }

    #[test]
    fn dilation_spreads_the_largest_level() {
        let mut levels = ShadowGrid::new(5, 5, 4);
        levels.set(2, 2, 6);
        levels.set(2, 1, 3);
        dilate_shadows(&mut levels);
        assert_eq!(
            levels.get(1, 1),
            6,
            "inner ring takes the brighter neighbour"
        );
        assert_eq!(levels.get(0, 0), 6, "outer ring follows");
        assert_eq!(levels.get(2, 2), 6, "interior untouched");
    }

    #[test]
    fn dilation_handles_tiny_grids() {
        for (w, h) in [(1, 1), (1, 5), (5, 1), (2, 2), (3, 3)] {
            let mut bits = grid(w, h, &[(0, 0)]);
            dilate_shadows(&mut bits);
            assert!(lit(&bits, 0, 0));
        }
    }
}
