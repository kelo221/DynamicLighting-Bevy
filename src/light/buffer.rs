//! The per-frame light buffer: what the packing system produced, the slots other slices fill in,
//! and the upload into the [`ShaderBuffer`] every lit material binds.

use bevy::asset::uuid_handle;
use bevy::ecs::entity::EntityHashMap;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;

use super::shape::LightShape;
use crate::shared::{
    BakeId, GpuLight, GpuLightGlobals, GpuReflectionProbes, GpuSkyIrradiance, GpuSunShadow,
    frame_shader_buffer, write_shader_buffer,
};

/// The light buffer every `DynLitMaterial` binds. Its layout is the WESL `DynamicLights`: the
/// [`GpuLightGlobals`] header, the sky's [`GpuSkyIrradiance`], the sun's [`GpuSunShadow`], the
/// [`GpuReflectionProbes`], then the baked slots and the realtime lights.
pub const LIGHT_BUFFER: Handle<ShaderBuffer> = uuid_handle!("5c3a3f0e-4d8b-4f6e-9a51-7d2c0b6e1f42");

/// Baked lights in bake order, provided by the baked-lighting slice when a bake loads. Slot `i`
/// is light `i` in the buffer; the triangle data refers to lights by slot. Distant lights
/// (suns) come last, after the lights of the light BVH.
#[derive(Resource, Default, Clone, Debug)]
pub struct BakedLightSlots {
    pub slots: Vec<BakedSlot>,
}

impl BakedLightSlots {
    /// The distant lights at the end of the slots.
    pub fn distant_count(&self) -> usize {
        self.slots
            .iter()
            .rev()
            .take_while(|slot| slot.shape.is_distant())
            .count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakedSlot {
    pub id: BakeId,
    /// Where the light was baked; a light that moved away is evaluated as realtime instead.
    /// Distant lights have no position, so only their shape counts.
    pub origin: Vec3,
    /// The radius the light was baked with. The triangle light lists, the light BVH and the
    /// distance cube end there, so a light that grows past it is evaluated as realtime instead.
    pub radius: f32,
    /// The area light's shape or the sun's disk its soft shadows were traced for.
    pub shape: LightShape,
    /// Bits per pixel of the light's bounce data, 1 to 8.
    pub bounce_bits_per_pixel: u32,
    /// The bounce data is colored: [`Self::bounce_bits_per_pixel`] per channel.
    pub colored_bounce: bool,
    /// Bits per texel of the light's baked shadows: 1, or more for soft area-light shadows.
    pub shadow_bits_per_texel: u32,
}

/// Texture-array slots assigned to lights by other slices before packing.
#[derive(Resource, Default, Debug)]
pub struct LightAttachments {
    /// Slot in the cookie texture array.
    pub cookies: EntityHashMap<u32>,
    /// Slot in the realtime shadow cubemap array.
    pub shadow_cubemaps: EntityHashMap<u32>,
    /// The sun with realtime shadow cascades.
    pub sun_cascades: Option<Entity>,
}

/// Lights of which only one is used, chosen before the other slices attach theirs.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct LightChoices {
    /// The sky light that lights the scene: the first by `LightRank` among the visible, valid
    /// sky lights. The others aren't packed.
    pub sky: Option<Entity>,
}

/// The packed light buffer for this frame, and which entity is in which slot.
#[derive(Resource, Default, Debug)]
pub struct PackedLights {
    pub globals: GpuLightGlobals,
    /// The sky's diffuse light, zero without a sky.
    pub sky: GpuSkyIrradiance,
    /// The cascades of the sun with realtime shadows, empty without one.
    pub sun_shadow: GpuSunShadow,
    /// The reflection probes, none without any.
    pub probes: GpuReflectionProbes,
    /// Baked slots followed by realtime lights.
    pub lights: Vec<GpuLight>,
    /// The entity in each baked slot; `None` for disabled slots.
    pub baked: Vec<Option<Entity>>,
    /// The entity behind each realtime light, in buffer order.
    pub realtime: Vec<Entity>,
    /// The camera the realtime lights were chosen for: the active camera with the highest order.
    pub viewer: Option<RealtimeViewer>,
}

/// The camera that chose the realtime lights, in the matrices its view uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RealtimeViewer {
    pub view_from_world: Mat4,
    pub clip_from_view: Mat4,
}

impl PackedLights {
    /// The buffer contents. Always holds at least one light record, because a WGSL runtime-sized
    /// array can't be bound empty.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.parts().concat()
    }

    /// The buffer contents in five parts: the globals, the sky, the sun's cascades, the probes,
    /// then the lights.
    pub fn parts(&self) -> [&[u8]; 5] {
        let lights = if self.lights.is_empty() {
            bytemuck::bytes_of(&GpuLight::DISABLED)
        } else {
            bytemuck::cast_slice(&self.lights)
        };
        [
            bytemuck::bytes_of(&self.globals),
            bytemuck::bytes_of(&self.sky),
            bytemuck::bytes_of(&self.sun_shadow),
            bytemuck::bytes_of(&self.probes),
            lights,
        ]
    }
}

/// Creates the light buffer asset; a no-op without a renderer.
pub(super) fn create_light_buffer(app: &mut App) {
    let Some(mut buffers) = app.world_mut().get_resource_mut::<Assets<ShaderBuffer>>() else {
        return;
    };
    let buffer = frame_shader_buffer(&PackedLights::default().to_bytes(), "dynlight lights");
    buffers
        .insert(&LIGHT_BUFFER, buffer)
        .expect("the light buffer handle is a fixed UUID");
}

/// Copies the packed lights into the GPU buffer.
pub(super) fn upload_lights(
    packed: Res<PackedLights>,
    buffers: Option<ResMut<Assets<ShaderBuffer>>>,
) {
    let Some(mut buffers) = buffers else {
        return;
    };
    let Some(mut buffer) = buffers.get_mut(&LIGHT_BUFFER) else {
        return;
    };
    write_shader_buffer(&mut buffer, &packed.parts(), 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::{
        GPU_LIGHT_GLOBALS_SIZE, GPU_LIGHT_SIZE, GPU_REFLECTION_PROBES_SIZE, GPU_SKY_SIZE,
        GPU_SUN_SHADOW_SIZE,
    };

    const SKY_END: usize = GPU_LIGHT_GLOBALS_SIZE + GPU_SKY_SIZE;
    const SUN_END: usize = SKY_END + GPU_SUN_SHADOW_SIZE;
    const HEADER: usize = SUN_END + GPU_REFLECTION_PROBES_SIZE;

    #[test]
    fn empty_buffer_still_holds_one_disabled_light() {
        let bytes = PackedLights::default().to_bytes();
        assert_eq!(bytes.len(), HEADER + GPU_LIGHT_SIZE);
        let light: GpuLight = bytemuck::pod_read_unaligned(&bytes[HEADER..]);
        assert_eq!(light, GpuLight::DISABLED);
    }

    #[test]
    fn lights_follow_the_header() {
        let packed = PackedLights {
            globals: GpuLightGlobals {
                realtime_count: 2,
                ..default()
            },
            sky: GpuSkyIrradiance { sh: [[0.5; 4]; 9] },
            sun_shadow: GpuSunShadow {
                count: 3,
                ..default()
            },
            probes: GpuReflectionProbes {
                count: 2,
                ..default()
            },
            lights: vec![GpuLight::default(); 2],
            ..default()
        };
        let bytes = packed.to_bytes();
        assert_eq!(bytes.len(), HEADER + 2 * GPU_LIGHT_SIZE);
        let globals: GpuLightGlobals =
            bytemuck::pod_read_unaligned(&bytes[..GPU_LIGHT_GLOBALS_SIZE]);
        assert_eq!(globals.realtime_count, 2);
        let sky: GpuSkyIrradiance =
            bytemuck::pod_read_unaligned(&bytes[GPU_LIGHT_GLOBALS_SIZE..SKY_END]);
        assert_eq!(sky, packed.sky);
        let sun_shadow: GpuSunShadow = bytemuck::pod_read_unaligned(&bytes[SKY_END..SUN_END]);
        assert_eq!(sun_shadow.count, 3);
        let probes: GpuReflectionProbes = bytemuck::pod_read_unaligned(&bytes[SUN_END..HEADER]);
        assert_eq!(probes.count, 2);
    }
}
