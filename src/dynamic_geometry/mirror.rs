//! CPU mirror of `cube_area_shadow` in `shaders/distance_cubes.wesl`: soft shadows of rectangle
//! and tube lights on meshes without bake data, from the distance cube baked at the light's
//! center.
//!
//! A light point offset by `o` from the center sees a blocker a fraction `s` of the way to the
//! receiver along the receiver's direction `d` where the center sees `d + o · (1 − s) / s`. A
//! blocker search over the shape gives the mean fraction (each blocker's distance over the
//! receiver plane's along the same ray, exact for blockers parallel to the receiver), then a
//! filter over the shape scaled by `(1 − s) / s` gives the share of the light that reaches the
//! receiver: sharp where a blocker touches the receiver, wider the farther it is.
//!
//! Every lookup blends the depth tests of four texels bilinearly, as Henry's angular filter
//! does, so the result changes smoothly as the receiver moves:
//! - The search's blocked share and mean distance ramp as blockers enter and leave it, and the
//!   shadow fades out as the last ones leave.
//! - Filter taps read a grid as wide as their spacing, so each tap's ramp meets the next and
//!   penumbras have no steps. The grids are fixed per level like a cube's mips (the centers of
//!   1, 2, 4, … texel wide blocks, which line up across face seams), and a tap blends the two
//!   levels around the spacing. Each grid point tests the texel it falls in along that texel's
//!   own ray, the ray its distance was traced along.

use bevy::math::{Vec2, Vec3};

use crate::shared::cube::{
    direction_to_face_uv, direction_to_texel, face_uv_to_direction, texel_direction, texel_index,
};

/// Bias added to stored distances by the `DistanceCubes` filter (Henry's +0.25 m).
pub const DISTANCE_CUBE_BIAS: f32 = 0.25;
/// Cube texels per face side.
pub const CUBE_RESOLUTION: u32 = 64;
/// Bilinear lookups (four texels each) of the blocker search and of the filter.
pub const BLOCKER_TAPS: u32 = 8;
pub const FILTER_TAPS: u32 = 8;
/// How far the blocker search reaches, and the widest penumbra, relative to the shape: blockers
/// at least a quarter of the way from the light to the receiver. Nearer ones soften as if they
/// were a quarter of the way.
pub const SEARCH_SCALE: f32 = 3.0;
/// Half a cube texel at a face's center, as a tangent: the smallest spread of the lookups, so a
/// thin penumbra still blends neighbouring texels.
pub const TEXEL_SPREAD: f32 = 1.0 / CUBE_RESOLUTION as f32;
/// The fractional part of the golden ratio, which spreads the taps across the shape's height.
pub const GOLDEN_FRACTION: f32 = 0.618_034;
/// The coarsest filter grid, as `log2` of its cell in texels (16 texels, a quarter face).
pub const MAX_FILTER_LEVEL: f32 = 4.0;

/// The flat cube texel a direction falls in (`direction_to_index`).
pub fn direction_to_index(direction: Vec3) -> u32 {
    let (face, x, y) = direction_to_texel(direction, CUBE_RESOLUTION);
    texel_index(face, x, y, CUBE_RESOLUTION)
}

/// `cube_texel`: the texel a direction falls in, as its flat index and the ray through its
/// center.
fn cube_texel(direction: Vec3) -> (u32, Vec3) {
    let (face, x, y) = direction_to_texel(direction, CUBE_RESOLUTION);
    let ray = texel_direction(face, x, y, CUBE_RESOLUTION);
    (texel_index(face, x, y, CUBE_RESOLUTION), ray)
}

/// What the shaders know about an area light: its center and unit axes, the half extents
/// along them (a tube's `half.y` is 0), and its radius around the center, which its cube
/// stores where rays hit nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubeArea {
    pub center: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub half: Vec2,
    pub radius: f32,
}

/// A receiver: world position and unit normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Receiver {
    pub world: Vec3,
    pub normal: Vec3,
}

/// The receiver's depth test along one lookup: Henry's angular filter compares the stored
/// distance with the distance to the receiver's plane along the lookup, or near grazing angles
/// with the receiver's own distance. Lookups tilted away from the receiver can meet its plane
/// past the light's radius, where the cube stores the radius for empty space, so the plane
/// distance stops at the radius.
struct DepthTest {
    distance: f32,
    projected: f32,
    grazing: f32,
    bias: f32,
    radius: f32,
}

impl DepthTest {
    fn new(receiver: Receiver, outward: Vec3, distance: f32, radius: f32) -> Self {
        let ndotl = receiver.normal.dot(-outward / distance);
        let facing = ndotl.max(0.001);
        Self {
            distance,
            projected: distance * facing,
            grazing: (facing * 10.0).clamp(0.0, 1.0).powi(4),
            bias: 0.05 + 0.05 * (1.0 - ndotl),
            radius,
        }
    }

    /// Whether a stored distance along `direction` lets the light through (0 or 1), and the
    /// receiver's distance along it that the test compares with.
    fn lit(&self, normal: Vec3, direction: Vec3, stored: f32) -> (f32, f32) {
        let ray = direction.normalize();
        let plane = (self.projected / normal.dot(-ray).max(0.001)).min(self.radius);
        let stored = stored + self.bias;
        let sphere = if self.distance <= stored { 1.0 } else { 0.0 };
        let planar = if plane <= stored { 1.0 } else { 0.0 };
        let reference = self.distance + (plane - self.distance) * self.grazing;
        (sphere + (planar - sphere) * self.grazing, reference)
    }
}

/// `cube_area_shadow`: how much of `light` reaches `receiver`, 0 to 1. `stored(index)` reads the
/// light's cube texel.
pub fn cube_area_shadow(light: CubeArea, receiver: Receiver, stored: impl Fn(u32) -> f32) -> f32 {
    let outward = receiver.world - light.center;
    let distance = outward.length();
    if distance < 1.0e-4 {
        return 1.0;
    }
    let test = DepthTest::new(receiver, outward, distance, light.radius);
    let normal = receiver.normal;
    let minimum = Vec2::splat(distance * TEXEL_SPREAD);
    let toward = |tap: Vec2, spread: Vec2| {
        outward + light.x * (tap.x * spread.x) + light.y * (tap.y * spread.y)
    };

    let spread = (light.half * SEARCH_SCALE).max(minimum);
    let (mut blocked, mut fraction) = (0.0, 0.0);
    for k in 0..BLOCKER_TAPS {
        let direction = toward(lattice_tap(k, BLOCKER_TAPS), spread);
        let lookup = bilinear_lookup(&test, normal, direction, 1.0, &stored);
        blocked += 1.0 - lookup.x;
        fraction += lookup.y;
    }
    if blocked <= 0.0 {
        return 1.0;
    }
    let fraction = fraction / blocked;
    let scale = ((1.0 - fraction) / fraction.max(1.0e-3)).clamp(0.0, SEARCH_SCALE);
    let spread = (light.half * scale).max(minimum);
    let level = filter_level(spread, outward);
    let fine = level.floor();
    let blend = level - fine;
    let texels = (1u32 << fine as u32) as f32;
    // Each tap on the finer level, then the coarser one, as the shader orders them.
    let lit: f32 = (0..2 * FILTER_TAPS)
        .map(|k| {
            let coarse = k % 2 == 1;
            let weight = if coarse { blend } else { 1.0 - blend };
            if weight <= 0.0 {
                return 0.0;
            }
            let direction = toward(lattice_tap(k / 2, FILTER_TAPS), spread);
            let texels = if coarse { 2.0 * texels } else { texels };
            weight * bilinear_lookup(&test, normal, direction, texels, &stored).x
        })
        .sum();
    let lit = lit / FILTER_TAPS as f32;
    // Below one tap's worth of blockers, fade to lit as they leave the search.
    lit + (1.0 - lit) * (1.0 - blocked.min(1.0))
}

/// `filter_level`: `log2` of the taps' spacing across the shape in texels (as face uv near
/// `outward`), from 0 to [`MAX_FILTER_LEVEL`]. Along a tube it's `2 · spread / FILTER_TAPS`;
/// over a rectangle the taps are as far apart as squares sharing its area.
pub fn filter_level(spread: Vec2, outward: Vec3) -> f32 {
    let taps = FILTER_TAPS as f32;
    let spacing = (spread.max_element() / taps).max((spread.x * spread.y / taps).sqrt());
    let texels = spacing / outward.abs().max_element() * CUBE_RESOLUTION as f32;
    texels.max(1.0).log2().min(MAX_FILTER_LEVEL)
}

/// `bilinear_lookup`: the depth tests at the four centers of `texels` wide blocks around
/// `direction`, blended bilinearly: the lit share, and the blocked share times the blocker's
/// fraction of the way to the receiver. Blocks of one texel read the four nearest texels.
fn bilinear_lookup(
    test: &DepthTest,
    normal: Vec3,
    direction: Vec3,
    texels: f32,
    stored: &impl Fn(u32) -> f32,
) -> Vec2 {
    let (face, uv) = direction_to_face_uv(direction);
    let grid = uv * (CUBE_RESOLUTION as f32 / texels) - 0.5;
    let corner = grid.floor();
    let weight = grid - corner;
    let mut sum = Vec2::ZERO;
    for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let at = (corner + Vec2::new(x as f32 + 0.5, y as f32 + 0.5)) * texels;
        let (index, ray) = cube_texel(face_uv_to_direction(face, at / CUBE_RESOLUTION as f32));
        let wx = if x == 1 { weight.x } else { 1.0 - weight.x };
        let wy = if y == 1 { weight.y } else { 1.0 - weight.y };
        let value = stored(index);
        let (lit, reference) = test.lit(normal, ray, value);
        sum += Vec2::new(lit, (1.0 - lit) * value / reference) * (wx * wy);
    }
    sum
}

/// `lattice_tap`: tap `k` of `n` over the square `[−1, 1]²`, evenly spaced across x and spread
/// across y by the golden ratio (a Fibonacci lattice). Taps cover a rectangle uniformly, and a
/// tube's length evenly.
pub fn lattice_tap(k: u32, n: u32) -> Vec2 {
    let x = (k as f32 + 0.5) / n as f32;
    let y = (k as f32 * GOLDEN_FRACTION + 0.5).fract();
    Vec2::new(x, y) * 2.0 - 1.0
}

#[cfg(test)]
#[path = "mirror_tests.rs"]
mod tests;
