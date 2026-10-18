//! The shadowed sun's cascades in the light buffer, between the sky and the lights. The
//! realtime-shadows slice fits them to the camera each frame; the light slice packs them.

use bevy::prelude::*;
use bytemuck::{Pod, Zeroable};

/// Size in bytes of [`GpuSunShadow`]. The WESL `SunShadow` must match.
pub const GPU_SUN_SHADOW_SIZE: usize = 208;
/// The sun's shadow cascades, nearest the camera first.
pub const SUN_CASCADE_COUNT: usize = 3;

/// One cascade as the lit shader reads it: a world point's texture coordinates and depth are
/// `dot(row, (world, 1))` for the rows `u`, `v` and `depth`. Depth is 0 at the cascade's sunward
/// end and 1 at its far end.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuSunCascade {
    pub u: [f32; 4],
    pub v: [f32; 4],
    pub depth: [f32; 4],
    /// World size of one texel.
    pub texel_size: f32,
    /// Depth bias, in depth units.
    pub depth_bias: f32,
    pub _padding: [f32; 2],
}

/// The cascades of the one sun with realtime shadows. `count` is 0 without one.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuSunShadow {
    pub cascades: [GpuSunCascade; SUN_CASCADE_COUNT],
    pub count: u32,
    /// `FLAG_*` bits.
    pub flags: u32,
    pub _padding: [u32; 2],
}

impl GpuSunShadow {
    /// The cascades hold only meshes without bake data: the sun uses its baked shadows, so
    /// baked receivers aren't in them.
    pub const FLAG_MOVING_ONLY: u32 = 1;
}

/// This frame's [`GpuSunShadow`], kept by the realtime-shadows slice.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct SunShadowBlock(pub GpuSunShadow);

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;
    use crate::shared::wesl_check::{compile_shared_types, struct_layout};

    #[test]
    fn sun_shadow_layout_matches_wesl() {
        assert_eq!(size_of::<GpuSunCascade>(), 64);
        assert_eq!(size_of::<GpuSunShadow>(), GPU_SUN_SHADOW_SIZE);
        let module = compile_shared_types();
        let cascade = struct_layout(&module, "SunCascade");
        assert_eq!(cascade.size, 64);
        let names = ["u", "v", "depth", "texel_size", "depth_bias", "_padding"];
        let offsets = [0, 16, 32, 48, 52, 56];
        let expected: Vec<_> = names
            .iter()
            .map(|name| name.to_string())
            .zip(offsets)
            .collect();
        assert_eq!(cascade.members, expected);
        let shadow = struct_layout(&module, "SunShadow");
        assert_eq!(shadow.size as usize, GPU_SUN_SHADOW_SIZE);
        let names = ["cascades", "count", "flags", "_padding"];
        let expected: Vec<_> = names
            .iter()
            .map(|name| name.to_string())
            .zip([0, 192, 196, 200])
            .collect();
        assert_eq!(shadow.members, expected);
    }
}
