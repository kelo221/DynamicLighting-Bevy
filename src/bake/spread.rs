//! Points spread evenly over an area light's shape, for its soft-shadow rays.

use bevy::math::Vec2;

/// Point `index` of the spread over a rectangle (`rectangle`) or along a tube, as shape
/// coordinates from 0 to 1, shifted by `offset` and wrapped.
///
/// Over a rectangle it is the Sobol (0, 2) sequence (van der Corput's sequence across, Sobol's
/// second dimension down); along a tube van der Corput's alone, with `offset.y` as the unused
/// coordinate. The first `2^k` points split each axis into `2^k` equal strips with one point in
/// each, and the shift moves the strips along with the points. So any strip across the shape at
/// least twice that wide holds a point, wherever it lies: a thin occluder is seen once it hides
/// `2 / n` of the shape's width or height, after `n` points.
pub fn spread(index: u32, offset: Vec2, rectangle: bool) -> Vec2 {
    let across = unit(index.reverse_bits());
    let down = if rectangle {
        unit(sobol_second(index))
    } else {
        0.0
    };
    (Vec2::new(across, down) + offset).fract()
}

/// Sobol's second dimension (primitive polynomial `x + 1`): the XOR of the direction numbers
/// of the set bits of `index`, each the previous one XOR itself shifted right once.
fn sobol_second(mut index: u32) -> u32 {
    let (mut value, mut direction) = (0, 1u32 << 31);
    while index != 0 {
        if index & 1 != 0 {
            value ^= direction;
        }
        index >>= 1;
        direction ^= direction >> 1;
    }
    value
}

/// A 32-bit fraction as a float below 1.
fn unit(bits: u32) -> f32 {
    (bits >> 8) as f32 / (1u32 << 24) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The strip of `count` each of the first `count` points falls in, along `axis`.
    fn strips(count: u32, offset: Vec2, rectangle: bool, axis: usize) -> Vec<u32> {
        let mut strips: Vec<u32> = (0..count)
            .map(|index| (spread(index, offset, rectangle)[axis] * count as f32) as u32)
            .collect();
        strips.sort_unstable();
        strips
    }

    #[test]
    fn powers_of_two_put_one_point_in_each_strip() {
        for count in [1, 2, 4, 16, 64] {
            let all: Vec<u32> = (0..count).collect();
            assert_eq!(strips(count, Vec2::ZERO, true, 0), all, "across, {count}");
            assert_eq!(strips(count, Vec2::ZERO, true, 1), all, "down, {count}");
            assert_eq!(strips(count, Vec2::ZERO, false, 0), all, "tube, {count}");
        }
    }

    #[test]
    fn rectangles_put_one_point_in_each_cell_of_a_square_grid() {
        let mut cells: Vec<(u32, u32)> = (0..16)
            .map(|index| {
                let point = spread(index, Vec2::ZERO, true) * 4.0;
                (point.x as u32, point.y as u32)
            })
            .collect();
        cells.sort_unstable();
        let grid: Vec<(u32, u32)> = (0..4).flat_map(|x| (0..4).map(move |y| (x, y))).collect();
        assert_eq!(cells, grid);
    }

    #[test]
    fn shifted_points_leave_no_gap_wider_than_two_strips() {
        let offset = Vec2::new(0.371, 0.913);
        for axis in 0..2 {
            let mut values: Vec<f32> = (0..16).map(|i| spread(i, offset, true)[axis]).collect();
            values.sort_by(f32::total_cmp);
            let wrap = values[0] + 1.0 - values[15];
            let widest = values.windows(2).map(|w| w[1] - w[0]).fold(wrap, f32::max);
            assert!(widest <= 2.0 / 16.0 + 1e-6, "axis {axis}: {widest}");
        }
        let tube = spread(5, offset, false);
        assert_eq!(tube.y, offset.y, "a tube keeps its unused coordinate");
    }
}
