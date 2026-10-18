//! Picks the lights that get a shadow cubemap this frame, places the six faces of each cube at
//! its light (Henry's shadow camera, as one view per face) and lists the casters each face
//! draws. Only the faces that need it render; the others keep their last rendering in the cube
//! array. Faces are plain entities, not cameras: the render world draws them in its own pass.

use core::f32::consts::FRAC_PI_2;

use bevy::camera::CameraProjection;
use bevy::camera::primitives::{Aabb, Frustum};
use bevy::camera::visibility::{RenderLayers, SetViewVisibility};
use bevy::ecs::entity::EntityHashMap;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use bevy::render::renderer::RenderDevice;
use bevy::render::sync_world::SyncToRenderWorld;

use super::casters::{ActiveCube, Caster, face_lists};
use super::cubemaps::{ShadowCubemapCapacity, ensure_capacity, shadow_cubemap_format};
use super::faces::{
    POINT_BLUR, SHADOW_NEAR_PLANE, face_blur, face_orientation, select_shadow_lights,
};
use super::slots::{CubePose, CubeSlots};
use super::tracking::{CasterChanges, bounds};
use crate::baked_lighting::BakedReceiver;
use crate::light::{BakedInPlace, DynamicLight, LightAttachments, ShadowMode};
use crate::shared::cube::CUBE_FACE_COUNT;
use crate::shared::{DynamicGeometryMode, DynamicLightingSettings, RuntimeQuality, sees_sphere};

/// One face of a shadow cubemap: array layer `layer`, seen from its light at `pose`.
#[derive(Component, Clone, Debug, PartialEq)]
#[require(SyncToRenderWorld)]
pub struct ShadowFace {
    pub layer: u32,
    pub pose: CubePose,
    /// The face renders this frame.
    pub renders: bool,
    /// What it draws when it renders.
    pub casters: Vec<Entity>,
}

/// What the face pass reads for one face or cascade view: the WESL `ShadowFace`.
#[derive(Component, ShaderType, Clone, Copy, Debug)]
pub struct ShadowFaceUniform {
    pub radius: f32,
    pub layer: u32,
    pub blur: f32,
    /// 1 for the sun's cascades, whose depth is linear along the sun's rays; 0 for cube faces.
    pub orthographic: u32,
}

/// What the shadow cubemaps cost this frame.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShadowCubemapStats {
    /// Lights with a shadow cubemap.
    pub lights: u32,
    /// Cube faces rendered this frame; the others kept their last rendering.
    pub faces_rendered: u32,
    /// Cascades of the sun's realtime shadows rendered this frame.
    pub cascades_rendered: u32,
}

/// The face entities, six per cube slot, the light in each slot and the cubes rendering.
#[derive(Resource, Default, Debug)]
pub(super) struct ShadowFaces {
    pub faces: Vec<Entity>,
    slots: CubeSlots<Entity>,
    pub active: Vec<ActiveCube>,
}

/// Where face `face` of a cube at `pose` looks from, and its projection: 90° square, with
/// Bevy's infinite reverse-Z like any perspective camera.
pub(super) fn face_view(pose: CubePose, face: u32) -> (GlobalTransform, Mat4) {
    let (forward, up) = face_orientation(face);
    let transform = Transform::from_translation(pose.position).looking_to(forward, up);
    let projection = PerspectiveProjection {
        fov: FRAC_PI_2,
        aspect_ratio: 1.0,
        near: SHADOW_NEAR_PLANE,
        far: pose.radius.max(SHADOW_NEAR_PLANE * 2.0),
        ..default()
    };
    (transform.into(), projection.get_clip_from_view())
}

type ShadowLight<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Has<BakedInPlace>,
);
type Viewer<'a> = (&'a Camera, &'a GlobalTransform, &'a Frustum);

/// Gives the nearest visible `ShadowMode::Realtime` lights within the budget a cube slot,
/// decides which faces render again, places them at their lights and records the slots in
/// [`LightAttachments`].
#[allow(clippy::too_many_arguments)]
pub(super) fn place_shadow_faces(
    mut commands: Commands,
    settings: Res<DynamicLightingSettings>,
    lights: Query<ShadowLight>,
    viewers: Query<Viewer>,
    mut faces: Query<&mut ShadowFace>,
    mut placed: ResMut<ShadowFaces>,
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
        viewers
            .iter()
            .any(|(_, _, frustum)| sees_sphere(frustum, position, radius))
    };
    let candidates = lights
        .iter()
        // Suns have no position to see a cubemap from.
        .filter(|(_, light, _, visibility, _)| {
            light.shadows == ShadowMode::Realtime
                && !light.light_type.is_distant()
                && light.is_valid()
                && visibility.get()
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

    let placed = &mut *placed;
    placed.slots.assign(&selected);
    let format = shadow_cubemap_format(device.features());
    let recreated = ensure_capacity(
        placed.slots.needed(),
        &mut capacity,
        &mut images,
        budget,
        format,
    );
    let reset = recreated || changes.settle > 0;
    changes.settle = changes.settle.saturating_sub(1);
    let plan = placed.slots.plan(
        |entity| poses[&entity],
        &changes.changes,
        &changes.redraw,
        reset,
    );

    attachments.shadow_cubemaps.clear();
    placed.active.clear();
    for (slot, entry) in plan.iter().enumerate() {
        if let Some((entity, faces)) = *entry {
            attachments.shadow_cubemaps.insert(entity, slot as u32);
            if faces != 0 {
                let (slot, pose) = (slot as u32, poses[&entity]);
                placed.active.push(ActiveCube { slot, pose, faces });
            }
        }
    }

    // Where each face renders from this frame, if it renders.
    let face_count = CUBE_FACE_COUNT as usize;
    let active = &placed.active;
    let rendering = |layer: usize| {
        active
            .iter()
            .find(|cube| cube.slot as usize == layer / face_count)
            .filter(|cube| cube.faces & (1 << (layer % face_count)) != 0)
            .map(|cube| cube.pose)
    };
    for layer in placed.faces.len()..plan.len() * face_count {
        let pose = rendering(layer);
        let face = ShadowFace {
            layer: layer as u32,
            pose: pose.unwrap_or(IDLE_POSE),
            renders: pose.is_some(),
            casters: Vec::new(),
        };
        placed.faces.push(
            commands
                .spawn((Name::new("dynlight shadow face"), face))
                .id(),
        );
    }
    for (layer, &entity) in placed.faces.iter().enumerate() {
        let Ok(mut face) = faces.get_mut(entity) else {
            continue;
        };
        let pose = rendering(layer);
        face.renders = pose.is_some();
        if let Some(pose) = pose {
            face.pose = pose;
        }
    }
    stats.lights = poses.len() as u32;
    stats.faces_rendered = placed
        .active
        .iter()
        .map(|cube| cube.faces.count_ones())
        .sum();
}

/// The meshes the shadow views may draw, as [`CasterItem`]s.
pub(super) type CasterQuery<'a> = (
    Entity,
    &'a GlobalTransform,
    Option<&'a Aabb>,
    &'a InheritedVisibility,
    Option<&'a RenderLayers>,
    Has<BakedReceiver>,
);
pub(super) type CasterItem<'a> = (
    Entity,
    &'a GlobalTransform,
    Option<&'a Aabb>,
    &'a InheritedVisibility,
    Option<&'a RenderLayers>,
    bool,
);

/// A mesh as a shadow caster: its bounds, whether it is baked, and whether the shadow views see
/// it.
pub(super) fn caster_of(item: CasterItem) -> (Entity, Caster) {
    let (entity, transform, aabb, inherited, layers, baked) = item;
    let bounds = aabb.map(|aabb| {
        let sphere = bounds(transform, aabb, baked);
        (sphere.center, sphere.radius)
    });
    let shown =
        inherited.get() && layers.is_none_or(|layers| layers.intersects(&RenderLayers::default()));
    (
        entity,
        Caster {
            bounds,
            baked,
            shown,
        },
    )
}

/// Lists the casters of every face rendering this frame and marks them visible, so Bevy
/// extracts them even when no camera sees them.
pub(super) fn cull_casters(
    placed: Res<ShadowFaces>,
    mut faces: Query<&mut ShadowFace>,
    casters: Query<CasterQuery, With<Mesh3d>>,
    mut visibility: Query<&mut ViewVisibility>,
    mut found: Local<Vec<(Entity, Caster)>>,
) {
    if placed.active.is_empty() {
        return;
    }
    found.clear();
    found.extend(casters.iter().map(caster_of));
    let casters: Vec<Caster> = found.iter().map(|(_, caster)| *caster).collect();
    let lists = face_lists(&placed.active, &casters, placed.faces.len());
    for (list, &face) in lists.iter().zip(&placed.faces) {
        let Ok(mut face) = faces.get_mut(face) else {
            continue;
        };
        face.casters.clear();
        for &index in list {
            let entity = found[index as usize].0;
            face.casters.push(entity);
            if let Ok(mut visible) = visibility.get_mut(entity) {
                visible.set_visible();
            }
        }
    }
}

/// Where a face of a free slot waits.
const IDLE_POSE: CubePose = CubePose {
    position: Vec3::ZERO,
    radius: 1.0,
    moving_only: false,
    blur: POINT_BLUR,
};
