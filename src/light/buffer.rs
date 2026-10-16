//! The per-frame light buffer: what the packing system produced, the slots other slices fill in,
//! and the upload into the [`ShaderBuffer`] every lit material binds.

use bevy::asset::uuid_handle;
use bevy::ecs::entity::EntityHashMap;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;

use super::area::AreaLightShape;
use crate::shared::{BakeId, GpuLight, GpuLightGlobals};

/// The light buffer every `DynLitMaterial` binds. Its layout is the WESL `DynamicLights`: the
/// [`GpuLightGlobals`] header, then the baked slots, then the realtime lights.
pub const LIGHT_BUFFER: Handle<ShaderBuffer> = uuid_handle!("5c3a3f0e-4d8b-4f6e-9a51-7d2c0b6e1f42");

/// Baked lights in bake order, provided by the baked-lighting slice when a bake loads. Slot `i`
/// is light `i` in the buffer; the triangle data refers to lights by slot.
#[derive(Resource, Default, Clone, Debug)]
pub struct BakedLightSlots {
    pub slots: Vec<BakedSlot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakedSlot {
    pub id: BakeId,
    /// Where the light was baked; a light that moved away is evaluated as realtime instead.
    pub origin: Vec3,
    /// The radius the light was baked with. The triangle light lists, the light BVH and the
    /// distance cube end there, so a light that grows past it is evaluated as realtime instead.
    pub radius: f32,
    /// The shape an area light's soft shadows were traced for; `None` for point-like lights.
    pub area: Option<AreaLightShape>,
    /// Bits per pixel of the light's bounce data, 1 to 8.
    pub bounce_bits_per_pixel: u32,
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
}

/// The packed light buffer for this frame, and which entity is in which slot.
#[derive(Resource, Default, Debug)]
pub struct PackedLights {
    pub globals: GpuLightGlobals,
    /// Baked slots followed by realtime lights.
    pub lights: Vec<GpuLight>,
    /// The entity in each baked slot; `None` for disabled slots.
    pub baked: Vec<Option<Entity>>,
    /// The entity behind each realtime light, in buffer order.
    pub realtime: Vec<Entity>,
}

impl PackedLights {
    /// The buffer contents. Always holds at least one light record, because a WGSL runtime-sized
    /// array can't be bound empty.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = bytemuck::bytes_of(&self.globals).to_vec();
        if self.lights.is_empty() {
            bytes.extend_from_slice(bytemuck::bytes_of(&GpuLight::DISABLED));
        } else {
            bytes.extend_from_slice(bytemuck::cast_slice(&self.lights));
        }
        bytes
    }
}

/// Creates the light buffer asset; a no-op without a renderer.
pub(super) fn create_light_buffer(app: &mut App) {
    let Some(mut buffers) = app.world_mut().get_resource_mut::<Assets<ShaderBuffer>>() else {
        return;
    };
    let mut buffer = ShaderBuffer::from(PackedLights::default().to_bytes());
    buffer.label = "dynlight lights".into();
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
    buffer.clear();
    buffer.extend_from_slice(&packed.to_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::{GPU_LIGHT_GLOBALS_SIZE, GPU_LIGHT_SIZE};

    #[test]
    fn empty_buffer_still_holds_one_disabled_light() {
        let bytes = PackedLights::default().to_bytes();
        assert_eq!(bytes.len(), GPU_LIGHT_GLOBALS_SIZE + GPU_LIGHT_SIZE);
        let light: GpuLight = bytemuck::pod_read_unaligned(&bytes[GPU_LIGHT_GLOBALS_SIZE..]);
        assert_eq!(light, GpuLight::DISABLED);
    }

    #[test]
    fn lights_follow_the_header() {
        let packed = PackedLights {
            globals: GpuLightGlobals {
                realtime_count: 2,
                ..default()
            },
            lights: vec![GpuLight::default(); 2],
            ..default()
        };
        let bytes = packed.to_bytes();
        assert_eq!(bytes.len(), GPU_LIGHT_GLOBALS_SIZE + 2 * GPU_LIGHT_SIZE);
        let globals: GpuLightGlobals = bytemuck::pod_read_unaligned(&bytes[..32]);
        assert_eq!(globals.realtime_count, 2);
    }
}
