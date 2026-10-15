//! Collects the visible fog volumes every frame and uploads them for the fog pass.

use bevy::asset::uuid_handle;
use bevy::camera::primitives::{Frustum, Sphere};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::storage::ShaderBuffer;

use super::params::{Placement, bounding_radius, is_active, pack};
use super::volumetric::VolumetricLight;
use crate::effects::LightEffectState;
use crate::light::DynamicLight;
use crate::shared::GpuLight;

/// The fog volume records the fog pass reads.
pub const FOG_BUFFER: Handle<ShaderBuffer> = uuid_handle!("3e7b5d19-8c2a-4f06-b4e1-6d9a0c7f2b53");

/// Draws [`VolumetricLight`] fog on this camera. Fog needs scene depth, so this adds a
/// [`DepthPrepass`].
#[derive(Component, ExtractComponent, Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Component, Default, Debug, Clone, PartialEq)]
#[require(DepthPrepass)]
#[extract_app(RenderApp)]
pub struct FogPostProcess;

/// This frame's fog volumes, packed.
#[derive(Resource, Default, Debug)]
pub struct FogVolumes(pub Vec<GpuLight>);

impl FogVolumes {
    /// The buffer contents: at least one (disabled) record, since storage buffers can't be empty.
    pub fn to_bytes(&self) -> Vec<u8> {
        if self.0.is_empty() {
            bytemuck::bytes_of(&GpuLight::DISABLED).to_vec()
        } else {
            bytemuck::cast_slice(&self.0).to_vec()
        }
    }
}

type FogItem<'a> = (
    &'a DynamicLight,
    &'a VolumetricLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Option<&'a LightEffectState>,
);

/// Packs every active fog volume that some fog camera can see.
pub(super) fn pack_fog_volumes(
    lights: Query<FogItem>,
    cameras: Query<(&Camera, &Frustum), With<FogPostProcess>>,
    mut volumes: ResMut<FogVolumes>,
) {
    volumes.0.clear();
    if cameras.is_empty() {
        return;
    }
    for (light, fog, transform, visibility, effect) in &lights {
        let effect_intensity = effect.map_or(1.0, LightEffectState::intensity);
        if !visibility.get() || !is_active(light, fog, effect_intensity) {
            continue;
        }
        let (scale, rotation, position) = transform.to_scale_rotation_translation();
        let bounds = Sphere {
            center: position.into(),
            radius: bounding_radius(fog, light, scale),
        };
        let seen = cameras
            .iter()
            .any(|(camera, frustum)| camera.is_active && frustum.intersects_sphere(&bounds, true));
        if seen {
            let placement = Placement {
                position,
                rotation,
                scale,
            };
            volumes
                .0
                .push(pack(light, fog, effect_intensity, placement));
        }
    }
}

pub(super) fn create_fog_buffer(app: &mut App) {
    let Some(mut assets) = app.world_mut().get_resource_mut::<Assets<ShaderBuffer>>() else {
        return;
    };
    let mut buffer = ShaderBuffer::from(FogVolumes::default().to_bytes());
    buffer.label = "dynlight fog volumes".into();
    assets
        .insert(&FOG_BUFFER, buffer)
        .expect("the fog buffer handle is a fixed UUID");
}

pub(super) fn upload_fog_volumes(
    volumes: Res<FogVolumes>,
    assets: Option<ResMut<Assets<ShaderBuffer>>>,
) {
    let Some(mut assets) = assets else {
        return;
    };
    if let Some(mut buffer) = assets.get_mut(&FOG_BUFFER) {
        buffer.clear();
        buffer.extend_from_slice(&volumes.to_bytes());
    }
}
