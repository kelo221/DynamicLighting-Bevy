//! Picks the lights that get a shadow cubemap this frame and places six depth-only cameras at
//! each of them (Henry's shadow camera, as one camera per face). Only the faces that need it
//! render; the others keep their last rendering in the cube array.

use core::f32::consts::FRAC_PI_2;

use bevy::camera::RenderTarget;
use bevy::camera::primitives::{Frustum, Sphere};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::ecs::entity::EntityHashMap;
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::render::renderer::RenderDevice;
use bevy::render::sync_component::SyncComponent;

use super::cubemaps::{ShadowCubemapCapacity, ensure_capacity, shadow_cubemap_format};
use super::faces::{
    POINT_BLUR, SHADOW_CUBEMAP_RESOLUTION, SHADOW_NEAR_PLANE, face_blur, face_orientation,
    select_shadow_lights,
};
use super::slots::{CubePose, CubeSlots};
use super::tracking::CasterChanges;
use crate::light::{BakedInPlace, DynamicLight, LightAttachments, ShadowMode};
use crate::shared::cube::CUBE_FACE_COUNT;
use crate::shared::{DynamicGeometryMode, DynamicLightingSettings, RuntimeQuality};

/// Face cameras render before every other camera, each with its own order.
const FIRST_CAMERA_ORDER: isize = -1_000_000;

/// A camera rendering one face of a shadow cubemap; the render world turns its depth into
/// variance moments for array layer `layer`.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct ShadowFaceCamera {
    pub layer: u32,
    pub radius: f32,
    /// Draws only meshes without baked data, because its light uses its baked shadows.
    pub moving_only: bool,
    /// Gaussian blur of the face in texels ([`face_blur`]).
    pub blur: f32,
}

/// What the face pass reads for one face view: the WESL `ShadowFace`.
#[derive(Component, ShaderType, Clone, Copy, Debug)]
pub struct ShadowFaceUniform {
    pub radius: f32,
    pub layer: u32,
    pub blur: f32,
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
            blur: face.blur,
        })
    }
}

/// What the shadow cubemaps cost this frame.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShadowCubemapStats {
    /// Lights with a shadow cubemap.
    pub lights: u32,
    /// Cube faces rendered this frame; the others kept their last rendering.
    pub faces_rendered: u32,
}

/// The face cameras, six per cube slot, and the light in each slot.
#[derive(Resource, Default, Debug)]
pub(super) struct ShadowCameras {
    faces: Vec<Entity>,
    slots: CubeSlots<Entity>,
}

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

fn face_camera(index: usize, pose: CubePose, active: bool) -> impl Bundle {
    let transform = face_transform(pose.position, index as u32 % CUBE_FACE_COUNT);
    (
        Name::new("dynlight shadow face"),
        Camera3d::default(),
        Camera {
            order: FIRST_CAMERA_ORDER + index as isize,
            is_active: active,
            ..default()
        },
        // No color target: only the depth prepass runs.
        RenderTarget::None {
            size: UVec2::splat(SHADOW_CUBEMAP_RESOLUTION),
        },
        Msaa::Off,
        DepthPrepass,
        face_projection(pose.radius),
        transform,
        GlobalTransform::from(transform),
        ShadowFaceCamera {
            layer: index as u32,
            radius: pose.radius,
            moving_only: pose.moving_only,
            blur: pose.blur,
        },
    )
}

type ShadowLight<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Has<BakedInPlace>,
);
type Viewer<'a> = (&'a Camera, &'a GlobalTransform, &'a Frustum);
type FaceItem<'a> = (
    &'a mut ShadowFaceCamera,
    &'a mut Camera,
    &'a mut Transform,
    &'a mut GlobalTransform,
    &'a mut Projection,
);

/// Gives the nearest visible `ShadowMode::Realtime` lights within the budget a cube slot,
/// activates the face cameras whose faces must render again, moves them to their lights and
/// records the slots in [`LightAttachments`].
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
    mut changes: ResMut<CasterChanges>,
    mut stats: ResMut<ShadowCubemapStats>,
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
        .filter(|(_, light, _, visibility, _)| {
            light.shadows == ShadowMode::Realtime && light.is_valid() && visibility.get()
        })
        .map(|(entity, light, transform, ..)| (entity, transform.translation(), light.radius))
        .filter(|&(_, position, radius)| seen(position, radius))
        .map(|(entity, position, _)| (entity, position))
        .collect();
    let selected = select_shadow_lights(candidates, viewer, budget as usize);
    let Some(device) = device else {
        return;
    };
    // Lights that use their baked shadows only need the meshes the bake doesn't cover, unless
    // moving meshes ignore the bake too.
    let use_bake = settings.dynamic_geometry != DynamicGeometryMode::LightingOnly;
    let poses: EntityHashMap<CubePose> = selected
        .iter()
        .filter_map(|&entity| lights.get(entity).ok())
        .map(|(entity, light, transform, _, in_place)| {
            let pose = CubePose {
                position: transform.translation(),
                radius: light.radius,
                moving_only: in_place && use_bake,
                blur: face_blur(light.extent(), light.radius),
            };
            (entity, pose)
        })
        .collect();

    let cameras = &mut *cameras;
    cameras.slots.assign(&selected);
    let format = shadow_cubemap_format(device.features());
    let recreated = ensure_capacity(
        cameras.slots.needed(),
        &mut capacity,
        &mut images,
        budget,
        format,
    );
    let reset = recreated || changes.settle > 0;
    changes.settle = changes.settle.saturating_sub(1);
    let plan = cameras
        .slots
        .plan(|entity| poses[&entity], &changes.changes, reset);

    attachments.shadow_cubemaps.clear();
    for (slot, entry) in plan.iter().enumerate() {
        if let Some((entity, _)) = entry {
            attachments.shadow_cubemaps.insert(*entity, slot as u32);
        }
    }

    let face_count = CUBE_FACE_COUNT as usize;
    let placement = |index: usize| {
        plan.get(index / face_count)
            .copied()
            .flatten()
            .map(|(entity, faces)| (poses[&entity], faces & (1 << (index % face_count)) != 0))
    };
    for index in cameras.faces.len()..plan.len() * face_count {
        let (pose, active) = placement(index).unwrap_or((IDLE_POSE, false));
        let camera = commands.spawn(face_camera(index, pose, active)).id();
        cameras.faces.push(camera);
    }
    for (index, &entity) in cameras.faces.iter().enumerate() {
        let Ok((mut face, mut camera, mut transform, mut global, mut projection)) =
            faces.get_mut(entity)
        else {
            continue;
        };
        let Some((pose, true)) = placement(index) else {
            if camera.is_active {
                camera.is_active = false;
            }
            continue;
        };
        if !camera.is_active {
            camera.is_active = true;
        }
        let placed = face_transform(pose.position, index as u32 % CUBE_FACE_COUNT);
        if *transform != placed {
            *transform = placed;
            *global = GlobalTransform::from(placed);
        }
        if face.radius != pose.radius {
            face.radius = pose.radius;
            *projection = face_projection(pose.radius);
        }
        if face.moving_only != pose.moving_only {
            face.moving_only = pose.moving_only;
        }
        if face.blur != pose.blur {
            face.blur = pose.blur;
        }
    }
    *stats = ShadowCubemapStats {
        lights: poses.len() as u32,
        faces_rendered: plan
            .iter()
            .flatten()
            .map(|(_, faces)| faces.count_ones())
            .sum(),
    };
}

/// Where a face camera of a free slot waits.
const IDLE_POSE: CubePose = CubePose {
    position: Vec3::ZERO,
    radius: 1.0,
    moving_only: false,
    blur: POINT_BLUR,
};
