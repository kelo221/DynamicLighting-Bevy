//! The sky's diffuse light in the light buffer, between the globals and the lights. The
//! environment slice computes it from the sky's image; the light slice packs it.

use bevy::prelude::*;
use bytemuck::{Pod, Zeroable};

/// Size in bytes of [`GpuSkyIrradiance`]. The WESL `SkyIrradiance` must match.
pub const GPU_SKY_SIZE: usize = 144;

/// The sky's irradiance over π in the sky's own frame, as spherical harmonics of bands 0 to 2:
/// coefficient `i` per color channel in `sh[i][..3]`, in the order of
/// `environment::sh::basis`. A surface facing `n` (in the sky's frame) receives
/// `Σ sh[i] · Y_i(n)` times the sky light's color and intensity. Zero without a sky.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuSkyIrradiance {
    pub sh: [[f32; 4]; 9],
}

/// The current sky's [`GpuSkyIrradiance`], kept by the environment slice.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct SkyIrradiance(pub GpuSkyIrradiance);

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use super::*;
    use crate::shared::GPU_LIGHT_GLOBALS_SIZE;
    use crate::shared::wesl_check::{compile_shared_types, struct_layout};

    #[test]
    fn sky_layout_matches_wesl() {
        assert_eq!(size_of::<GpuSkyIrradiance>(), GPU_SKY_SIZE);
        let module = compile_shared_types();
        let wesl = struct_layout(&module, "SkyIrradiance");
        assert_eq!(wesl.size as usize, GPU_SKY_SIZE);
        assert_eq!(wesl.members, vec![("sh".to_string(), 0)]);
    }

    #[test]
    fn the_sky_and_the_lights_follow_the_globals() {
        let module = compile_shared_types();
        let buffer = struct_layout(&module, "DynamicLights");
        let sky = GPU_LIGHT_GLOBALS_SIZE as u32;
        assert_eq!(
            buffer.members,
            vec![
                ("globals".to_string(), 0),
                ("sky".to_string(), sky),
                ("lights".to_string(), sky + GPU_SKY_SIZE as u32),
            ]
        );
    }
}
