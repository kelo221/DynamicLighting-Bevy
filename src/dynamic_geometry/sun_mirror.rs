//! CPU mirror of `sun_map_shadow` in `shaders/sun_map.wesl`: how much of a baked sun's light the
//! static scene lets through to a point on a mesh without bake data.

use bevy::math::{Vec2, Vec3};

use crate::bake::SunMap;

/// Meters, and map texels, a receiver may lie behind the stored surface and still be lit.
pub const SUN_MAP_BIAS: f32 = 0.05;
pub const SUN_MAP_TEXEL_BIAS: f32 = 2.0;

/// The share of the sun's light reaching `world`, 0 to 1: the depth tests of the 4×4 texels
/// around it, weighted like four overlapping 3×3 averages blended bilinearly. 1 off the map.
pub fn sun_map_shadow(map: &SunMap, world: Vec3) -> f32 {
    let [rx, ry, rz] = map.rows;
    let m = Vec3::new(
        rx.truncate().dot(world) + rx.w,
        ry.truncate().dot(world) + ry.w,
        rz.truncate().dot(world) + rz.w,
    );
    if m.x < 0.0 || m.y < 0.0 || m.x > 1.0 || m.y > 1.0 {
        return 1.0;
    }
    let size = map.resolution as f32;
    let texel_meters = 1.0 / (rx.truncate().length() * size).max(1.0e-20);
    let bias = (SUN_MAP_BIAS + SUN_MAP_TEXEL_BIAS * texel_meters) * rz.truncate().length();
    let depth = m.z - bias;
    let grid = m.truncate() * size - 0.5;
    let base = grid.floor();
    let w = grid - base;
    let lit: f32 = (0..4)
        .map(|y| {
            let row: f32 = (0..4)
                .map(|x| {
                    let texel = base + Vec2::new(x as f32, y as f32) - 1.0;
                    weight(x, w.x) * texel_lit(map, texel, depth)
                })
                .sum();
            weight(y, w.y) * row
        })
        .sum();
    lit / 9.0
}

/// Texel `i`'s weight, times 3, from one before the bilinear footprint to one after.
fn weight(i: u32, w: f32) -> f32 {
    match i {
        0 => 1.0 - w,
        3 => w,
        _ => 1.0,
    }
}

/// The stored map-space depth at texel `(x, y)`.
pub fn sun_map_depth(map: &SunMap, x: u32, y: u32) -> f32 {
    let index = (y * map.resolution + x) as usize;
    let word = map.texels[index / 2] >> (16 * (index % 2));
    (word & 0xFFFF) as f32 / 65535.0
}

fn texel_lit(map: &SunMap, grid: Vec2, depth: f32) -> f32 {
    let last = (map.resolution - 1) as f32;
    let texel = grid.clamp(Vec2::ZERO, Vec2::splat(last));
    let stored = sun_map_depth(map, texel.x as u32, texel.y as u32);
    if depth <= stored || stored >= 1.0 {
        1.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec4;

    use super::*;
    use crate::bake::{SunMap, pack_sun_depth};

    /// A 4² map over x, z ∈ [0, 4] for a sun overhead, depth 0 at y = 10 and 1 at y = 0, with a
    /// 1 m roof at y = 5 over the texels with x < 2.
    fn roofed() -> SunMap {
        let rows = [
            Vec4::new(0.25, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.25, 0.0),
            Vec4::new(0.0, -0.1, 0.0, 1.0),
        ];
        let depths: Vec<f32> = (0..16).map(|i| if i % 4 < 2 { 0.5 } else { 1.0 }).collect();
        let texels = depths
            .chunks(2)
            .map(|pair| pack_sun_depth(pair[0], pair[1]))
            .collect();
        SunMap {
            rows,
            resolution: 4,
            texels,
        }
    }

    #[test]
    fn roofs_shade_what_is_below_them() {
        let map = roofed();
        assert_eq!(sun_map_shadow(&map, Vec3::new(0.5, 1.0, 2.0)), 0.0, "under");
        assert_eq!(sun_map_shadow(&map, Vec3::new(0.5, 7.0, 2.0)), 1.0, "above");
        assert_eq!(
            sun_map_shadow(&map, Vec3::new(3.5, 1.0, 2.0)),
            1.0,
            "beside"
        );
        assert_eq!(
            sun_map_shadow(&map, Vec3::new(-1.0, 1.0, 2.0)),
            1.0,
            "off the map"
        );
        // Below the far plane but under open sky is lit.
        assert_eq!(sun_map_shadow(&map, Vec3::new(3.5, -2.0, 2.0)), 1.0);
    }

    #[test]
    fn edges_blend_over_three_texels_and_resting_receivers_stay_lit() {
        let map = roofed();
        let at = |x: f32| sun_map_shadow(&map, Vec3::new(x, 1.0, 2.0));
        assert!((at(2.0) - 0.5).abs() < 1.0e-6, "the edge is half lit");
        let ramp = [0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5].map(at);
        assert!(ramp.windows(2).all(|pair| pair[0] < pair[1]), "{ramp:?}");
        assert_eq!((ramp[0], ramp[6]), (0.0, 1.0));
        // Touching the roof's top, within the bias, is lit.
        assert_eq!(sun_map_shadow(&map, Vec3::new(0.5, 5.0, 2.0)), 1.0);
        assert_eq!(sun_map_shadow(&map, Vec3::new(0.5, 2.0, 2.0)), 0.0);
    }
}
