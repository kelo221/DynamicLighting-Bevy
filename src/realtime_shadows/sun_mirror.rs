//! CPU reference for `sun_vsm.wesl`: where a receiver falls in the sun's cascades and how much
//! of the sun reaches it.

use bevy::math::{Vec2, Vec3, Vec4};

use super::mirror::{MIN_BLEEDING_CUT, terminator};
use crate::shared::{GpuSunCascade, GpuSunShadow, SUN_CASCADE_COUNT};

/// Receivers are looked up this many texels off their surface along the normal.
pub const SUN_NORMAL_OFFSET: f32 = 1.5;
/// The rim of a cascade, in units of its half side, which its blur reads past.
pub const SUN_CASCADE_MARGIN: f32 = 0.02;
/// A cascade fades into the next over this share of its half side inside the rim.
pub const SUN_CASCADE_BLEND: f32 = 0.1;

/// Where a receiver falls in a cascade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CascadePoint {
    pub uv: Vec2,
    pub depth: f32,
    /// 0 at the cascade's center, 1 at its edge.
    pub rim: f32,
}

pub fn cascade_point(cascade: &GpuSunCascade, world: Vec3, normal: Vec3) -> CascadePoint {
    let point = (world + normal * (cascade.texel_size * SUN_NORMAL_OFFSET)).extend(1.0);
    let row = |row: [f32; 4]| Vec4::from_array(row).dot(point);
    let uv = Vec2::new(row(cascade.u), row(cascade.v));
    CascadePoint {
        uv,
        // Past the far end lies behind everything the cascade holds.
        depth: row(cascade.depth).min(1.0),
        rim: 2.0 * (uv - 0.5).abs().max_element(),
    }
}

/// The Chebyshev bound of a receiver at `depth` with filtered `moments`, past `bias`, with the
/// light-bleeding cut.
pub fn sun_visibility(moments: Vec2, depth: f32, bias: f32) -> f32 {
    if depth - bias <= moments.x {
        return 1.0;
    }
    let variance = (moments.y - moments.x * moments.x).max(0.0);
    let d = moments.x - depth;
    let p = variance / (variance + d * d).max(1.0e-12);
    ((p - MIN_BLEEDING_CUT) / (1.0 - MIN_BLEEDING_CUT)).clamp(0.0, 1.0)
}

fn linstep(low: f32, high: f32, value: f32) -> f32 {
    ((value - low) / (high - low).max(1.0e-6)).clamp(0.0, 1.0)
}

/// `sun_shadow` for a surface at `world` with unit `normal` and `cosine` towards the sun, where
/// `sample(cascade, uv)` gives the filtered moments of a cascade.
pub fn sun_shadow(
    shadow: &GpuSunShadow,
    world: Vec3,
    normal: Vec3,
    cosine: f32,
    static_receiver: bool,
    sample: impl Fn(usize, Vec2) -> Vec2,
) -> f32 {
    if shadow.count == 0 {
        return 1.0;
    }
    let mut fade = 1.0;
    if !static_receiver || shadow.flags & GpuSunShadow::FLAG_MOVING_ONLY == 0 {
        fade = terminator(cosine);
        if fade <= 0.0 {
            return 0.0;
        }
    }
    let count = (shadow.count as usize).min(SUN_CASCADE_COUNT);
    let inner = 1.0 - SUN_CASCADE_MARGIN;
    let shadowed = |index: usize, at: CascadePoint| {
        let cascade = &shadow.cascades[index];
        sun_visibility(sample(index, at.uv), at.depth, cascade.depth_bias)
    };
    for index in 0..count {
        let at = cascade_point(&shadow.cascades[index], world, normal);
        if at.rim >= inner {
            continue;
        }
        let lit = shadowed(index, at);
        let blend = linstep(inner - SUN_CASCADE_BLEND, inner, at.rim);
        if blend <= 0.0 {
            return fade * lit;
        }
        let next = if index + 1 < count {
            let there = cascade_point(&shadow.cascades[index + 1], world, normal);
            if there.rim < inner {
                shadowed(index + 1, there)
            } else {
                lit
            }
        } else {
            1.0
        };
        return fade * (lit + (next - lit) * blend);
    }
    fade
}
