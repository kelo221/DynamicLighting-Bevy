//! Picks the lights that get a shadow cubemap this frame and places six depth-only cameras at
//! each of them (Henry's shadow camera, as one camera per face).

use core::f32::consts::FRAC_PI_2;

use bevy::camera::RenderTarget;
use bevy::camera::primitives::{Frustum, Sphere};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::render::renderer::RenderDevice;
use bevy::render::sync_component::SyncComponent;

use super::cubemaps::{ShadowCubemapCapacity, ensure_capacity, shadow_cubemap_format};
use super::faces::{
    SHADOW_CUBEMAP_RESOLUTION, SHADOW_NEAR_PLANE, face_orientation, select_shadow_lights,
};
use crate::light::{DynamicLight, LightAttachments, ShadowMode};
use crate::shared::cube::CUBE_FACE_COUNT;
use crate::shared::{DynamicLightingSettings, RuntimeQuality};

/// Face cameras render before every other camera, each with its own order.
const FIRST_CAMERA_ORDER: isize = -1_000_000;

/// A camera rendering one face of a shadow cubemap; the render world turns its depth into
/// variance moments for array layer `layer`.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ShadowFaceCamera {
    pub layer: u32,
    pub radius: f32,
}

/// What the face pass reads for one face view.
#[derive(Component, ShaderType, Clone, Copy, Debug)]
pub struct ShadowFaceUniform {
    pub radius: f32,
    pub layer: u32,
}

impl SyncComponent<RenderApp> for ShadowFaceCamera {
    type Target = ShadowFaceUniform;
}

impl ExtractComponent<RenderApp> for ShadowFaceCamera {
    type QueryData = (&'static Self, &'static Camera);
    type QueryFilter = ();
    type Out = ShadowFaceUniform;

    fn extract_component((face, camera): QueryItem<'_, '_, Self::QueryData>) -> Option<Self::Out> {
        camera.is_active.then_some(ShadowFaceUniform {
            radius: face.radius,
            layer: face.layer,
        })
    }
}

/// The face cameras, six per cube slot; slots beyond this frame's lights are inactive.
#[derive(Resource, Default, Debug)]
pub(super) struct ShadowCameras(Vec<Entity>);

fn face_transform(position: Vec3, face: u32) -> Transform {
    let (forward, up) = face_orientation(face);
    Transform::from_translation(position).looking_to(forward, up)
}

/// Culling stops at the light's radius: nothing farther can come between it and what it lights.
fn face_projection(radius: f32) -> Projection {
    Projection::Perspective(PerspectiveProjection {
        fov: FRAC_PI_2,
        aspect_ratio: 1.0,
        near: SHADOW_NEAR_PLANE,
        far: radius.max(SHADOW_NEAR_PLANE * 2.0),
        ..default()
    })
}

fn face_camera(index: usize, position: Vec3, radius: f32) -> impl Bundle {
    let transform = face_transform(position, index as u32 % CUBE_FACE_COUNT);
    (
        Name::new("dynlight shadow face"),
        Camera3d::default(),
        Camera {
            order: FIRST_CAMERA_ORDER + index as isize,
            ..default()
        },
        // No color target: only the depth prepass runs.
        RenderTarget::None {
            size: UVec2::splat(SHADOW_CUBEMAP_RESOLUTION),
        },
        Msaa::Off,
        DepthPrepass,
        face_projection(radius),
        transform,
        GlobalTransform::from(transform),
        ShadowFaceCamera {
            layer: index as u32,
            radius,
        },
    )
}

type ShadowLight<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
);
type Viewer<'a> = (&'a Camera, &'a GlobalTransform, &'a Frustum);
type FaceItem<'a> = (
    &'a mut ShadowFaceCamera,
    &'a mut Camera,
    &'a mut Transform,
    &'a mut GlobalTransform,
    &'a mut Projection,
);

/// Assigns cube slots to the nearest visible `ShadowMode::Realtime` lights within the budget,
/// moves the face cameras to them and records the slots in [`LightAttachments`].
#[allow(clippy::too_many_arguments)]
pub(super) fn place_shadow_cameras(
    mut commands: Commands,
    settings: Res<DynamicLightingSettings>,
    lights: Query<ShadowLight, Without<ShadowFaceCamera>>,
    viewers: Query<Viewer, Without<ShadowFaceCamera>>,
    mut faces: Query<FaceItem>,
    mut cameras: ResMut<ShadowCameras>,
    mut attachments: ResMut<LightAttachments>,
    mut capacity: ResMut<ShadowCubemapCapacity>,
    mut images: ResMut<Assets<Image>>,
    device: Option<Res<RenderDevice>>,
) {
    let viewers: Vec<_> = viewers
        .iter()
        .filter(|(camera, ..)| camera.is_active)
        .collect();
    let viewer = viewers
        .iter()
        .max_by_key(|(camera, ..)| camera.order)
        .map(|(_, transform, _)| transform.translation());
    let budget = match settings.quality {
        RuntimeQuality::IntegratedGraphics => 0,
        _ => settings.shadow_cubemap_budget,
    };
    let seen = |position: Vec3, radius: f32| {
        let sphere = Sphere {
            center: position.into(),
            radius,
        };
        viewers
            .iter()
            .any(|(_, _, frustum)| frustum.intersects_sphere(&sphere, true))
    };
    let candidates = lights
        .iter()
        .filter(|(_, light, _, visibility)| {
            light.shadows == ShadowMode::Realtime && light.is_valid() && visibility.get()
        })
        .map(|(entity, light, transform, _)| (entity, transform.translation(), light.radius))
        .filter(|&(_, position, radius)| seen(position, radius))
        .map(|(entity, position, _)| (entity, position))
        .collect();
    let selected = select_shadow_lights(candidates, viewer, budget as usize);
    let Some(device) = device else {
        return;
    };
    let format = shadow_cubemap_format(device.features());
    ensure_capacity(
        selected.len() as u32,
        &mut capacity,
        &mut images,
        budget,
        format,
    );

    attachments.shadow_cubemaps.clear();
    let mut placements = Vec::with_capacity(selected.len());
    for (slot, &entity) in selected.iter().enumerate() {
        if let Ok((_, light, transform, _)) = lights.get(entity) {
            attachments.shadow_cubemaps.insert(entity, slot as u32);
            placements.push((transform.translation(), light.radius));
        }
    }

    let needed = placements.len() * CUBE_FACE_COUNT as usize;
    for index in cameras.0.len()..needed {
        let (position, radius) = placements[index / CUBE_FACE_COUNT as usize];
        cameras
            .0
            .push(commands.spawn(face_camera(index, position, radius)).id());
    }
    for (index, &entity) in cameras.0.iter().enumerate() {
        let Ok((mut face, mut camera, mut transform, mut global, mut projection)) =
            faces.get_mut(entity)
        else {
            continue;
        };
        let Some(&(position, radius)) = placements.get(index / CUBE_FACE_COUNT as usize) else {
            camera.is_active = false;
            continue;
        };
        camera.is_active = true;
        let placed = face_transform(position, index as u32 % CUBE_FACE_COUNT);
        if *transform != placed {
            *transform = placed;
            *global = GlobalTransform::from(placed);
        }
        if face.radius != radius {
            face.radius = radius;
            *projection = face_projection(radius);
        }
    }
}
