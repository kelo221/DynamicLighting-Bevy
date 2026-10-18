//! The header at the start of the light buffer: counts, ambient color and the shader clock.

use bytemuck::{Pod, Zeroable};

use super::settings::DebugView;

/// Size in bytes of [`GpuLightGlobals`]. The WESL `LightGlobals` must match, and the sky's
/// irradiance (`GpuSkyIrradiance`) starts right after it.
pub const GPU_LIGHT_GLOBALS_SIZE: usize = 48;

/// Scene-wide values the lighting shaders read once per fragment. Keeping them in the light
/// buffer saves a binding, and they change every frame, which a per-material uniform would make
/// expensive.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuLightGlobals {
    /// Linear RGB, already scaled by alpha² (see `DynamicLightingSettings::ambient_linear`).
    pub ambient: [f32; 3],
    /// Seconds since startup, our own clock. Bevy's `globals.time` wraps every hour.
    pub time: f32,
    /// Lights in baked slots, which come first in the array, in bake (light BVH) order.
    pub baked_count: u32,
    /// Realtime lights, which follow the baked slots.
    pub realtime_count: u32,
    /// `RuntimeQuality::level`. The lit shader uses the compiled `QUALITY` of the shader
    /// variant (`shared::variant`) instead; this copy is for readback checks.
    pub quality: u32,
    /// `GpuLightGlobals::FLAG_*` bits.
    pub flags: u32,
    /// Distant lights (suns) among the baked slots: the last `baked_distant` of them. The light
    /// BVH holds only the slots before them.
    pub baked_distant: u32,
    /// Distant lights among the realtime lights: the first `realtime_distant` of them. The
    /// realtime light grid holds only the lights after them.
    pub realtime_distant: u32,
    pub _padding: [u32; 2],
}

impl GpuLightGlobals {
    /// Dynamic geometry ignores the baked distance cubes (`DynamicGeometryMode::LightingOnly`).
    pub const FLAG_LIGHTING_ONLY: u32 = 1;
    /// Debug view: white surfaces without textures or emission, so only light shows.
    pub const FLAG_DEBUG_LIGHTING: u32 = 2;
    /// Debug view: a heat map of how many lights reach each fragment.
    pub const FLAG_DEBUG_LIGHT_COUNT: u32 = 4;

    /// The `FLAG_DEBUG_*` bit of `view`.
    pub const fn debug_flag(view: DebugView) -> u32 {
        match view {
            DebugView::None => 0,
            DebugView::Lighting => Self::FLAG_DEBUG_LIGHTING,
            DebugView::LightCount => Self::FLAG_DEBUG_LIGHT_COUNT,
        }
    }
}

#[cfg(test)]
mod tests {
    use core::mem::{offset_of, size_of};

    use super::*;
    use crate::shared::wesl_check::{compile_shared_types, struct_layout};

    #[test]
    fn globals_layout_matches_wesl() {
        assert_eq!(size_of::<GpuLightGlobals>(), GPU_LIGHT_GLOBALS_SIZE);
        let rust = [
            ("ambient", offset_of!(GpuLightGlobals, ambient)),
            ("time", offset_of!(GpuLightGlobals, time)),
            ("baked_count", offset_of!(GpuLightGlobals, baked_count)),
            (
                "realtime_count",
                offset_of!(GpuLightGlobals, realtime_count),
            ),
            ("quality", offset_of!(GpuLightGlobals, quality)),
            ("flags", offset_of!(GpuLightGlobals, flags)),
            ("baked_distant", offset_of!(GpuLightGlobals, baked_distant)),
            (
                "realtime_distant",
                offset_of!(GpuLightGlobals, realtime_distant),
            ),
            ("_padding", offset_of!(GpuLightGlobals, _padding)),
        ];
        let module = compile_shared_types();
        let wesl = struct_layout(&module, "LightGlobals");
        assert_eq!(wesl.size as usize, GPU_LIGHT_GLOBALS_SIZE);
        let wesl_members: Vec<(&str, usize)> = wesl
            .members
            .iter()
            .map(|(name, offset)| (name.as_str(), *offset as usize))
            .collect();
        assert_eq!(wesl_members, rust);
    }

    #[test]
    fn flags_match_wesl() {
        let source = include_str!("shaders/types.wesl");
        for (name, value) in [
            ("LIGHTING_ONLY", GpuLightGlobals::FLAG_LIGHTING_ONLY),
            ("DEBUG_LIGHTING", GpuLightGlobals::FLAG_DEBUG_LIGHTING),
            ("DEBUG_LIGHT_COUNT", GpuLightGlobals::FLAG_DEBUG_LIGHT_COUNT),
        ] {
            let line = format!("const GLOBALS_FLAG_{name}: u32 = {value}u;");
            assert!(source.contains(&line), "{line} is missing");
        }
        let views = DebugView::ALL.map(GpuLightGlobals::debug_flag);
        assert_eq!(views, [0, 2, 4]);
    }
}
