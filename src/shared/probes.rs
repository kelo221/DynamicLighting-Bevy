//! The reflection probes in the light buffer, between the sun's cascades and the lights. The
//! reflection-probes slice captures and sorts them; the light slice packs them.

use bevy::prelude::*;
use bytemuck::{Pod, Zeroable};

use super::sky::GpuSkyIrradiance;

/// The most reflection probes the lit shader reads.
pub const MAX_REFLECTION_PROBES: usize = 16;
/// Size in bytes of [`GpuReflectionProbe`]. The WESL `ReflectionProbe` must match.
pub const GPU_REFLECTION_PROBE_SIZE: usize = 208;
/// Size in bytes of [`GpuReflectionProbes`]. The WESL `ReflectionProbes` must match.
pub const GPU_REFLECTION_PROBES_SIZE: usize =
    16 + MAX_REFLECTION_PROBES * GPU_REFLECTION_PROBE_SIZE;

/// One probe as the lit shader reads it. A world point `p` lies in the probe's box when each of
/// `dot(row, (p, 1))` for the rows `x`, `y` and `z` is within ±1.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuReflectionProbe {
    pub x: [f32; 4],
    pub y: [f32; 4],
    pub z: [f32; 4],
    /// Where the probe's cube was captured, in world space.
    pub capture: [f32; 3],
    /// The probe's cube in the reflection cube array.
    pub layer: u32,
    /// Irradiance over π at the capture point, in world space, as for the sky.
    pub irradiance: GpuSkyIrradiance,
}

/// The probes, smallest box first, so the first box holding a point is the smallest one.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuReflectionProbes {
    pub count: u32,
    pub _padding: [u32; 3],
    pub probes: [GpuReflectionProbe; MAX_REFLECTION_PROBES],
}

impl Default for GpuReflectionProbes {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// This frame's [`GpuReflectionProbes`], kept by the reflection-probes slice.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct ReflectionProbeRecords(pub GpuReflectionProbes);

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;
    use crate::shared::wesl_check::{compile_shared_types, struct_layout};

    #[test]
    fn probe_layout_matches_wesl() {
        assert_eq!(size_of::<GpuReflectionProbe>(), GPU_REFLECTION_PROBE_SIZE);
        assert_eq!(size_of::<GpuReflectionProbes>(), GPU_REFLECTION_PROBES_SIZE);
        let module = compile_shared_types();
        let probe = struct_layout(&module, "ReflectionProbe");
        assert_eq!(probe.size as usize, GPU_REFLECTION_PROBE_SIZE);
        let names = ["x", "y", "z", "capture", "layer", "irradiance"];
        let expected: Vec<_> = names
            .iter()
            .map(|name| name.to_string())
            .zip([0, 16, 32, 48, 60, 64])
            .collect();
        assert_eq!(probe.members, expected);
        let probes = struct_layout(&module, "ReflectionProbes");
        assert_eq!(probes.size as usize, GPU_REFLECTION_PROBES_SIZE);
        let names = ["count", "_padding", "probes"];
        let expected: Vec<_> = names
            .iter()
            .map(|name| name.to_string())
            .zip([0, 4, 16])
            .collect();
        assert_eq!(probes.members, expected);
    }
}
