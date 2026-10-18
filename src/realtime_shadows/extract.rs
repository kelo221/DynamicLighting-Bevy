//! Hands every face and sun cascade rendering this frame to the render world as a view of its
//! own, run through the `Core3d` schedule before the cameras like Bevy's point-light shadow
//! views, with the casters it draws. Views that don't render lose their view, so nothing runs
//! for them.

use bevy::core_pipeline::Core3d;
use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::core_pipeline::schedule::RootNonCameraView;
use bevy::ecs::entity::EntityHashSet;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::render::Extract;
use bevy::render::sync_world::{MainEntity, RenderEntity};
use bevy::render::view::{ExtractedView, NoIndirectDrawing, RetainedViewEntity};

use super::cascades::SUN_CASCADE_RESOLUTION;
use super::faces::{POINT_BLUR, SHADOW_CUBEMAP_RESOLUTION};
use super::sun::SunCascadeView;
use super::tracking::ShadowTarget;
use super::views::{ShadowFace, ShadowFaceUniform, face_view};
use crate::shared::cube::CUBE_FACE_COUNT;

/// A face or cascade rendering this frame, in the render world: where it renders to and the
/// casters it draws.
#[derive(Component)]
pub(super) struct ShadowFaceView {
    pub target: ShadowTarget,
    pub casters: Vec<(Entity, MainEntity)>,
}

type FaceViewBundle = (
    ExtractedView,
    RootNonCameraView,
    NoIndirectDrawing,
    ShadowFaceUniform,
    ShadowFaceView,
);

/// One view to hand over.
struct Rendering<'a> {
    main_entity: Entity,
    world_from_view: GlobalTransform,
    clip_from_view: Mat4,
    resolution: u32,
    uniform: ShadowFaceUniform,
    target: ShadowTarget,
    casters: &'a [Entity],
}

fn insert_view(commands: &mut Commands, render_entity: Entity, rendering: Rendering) {
    let size = rendering.resolution;
    let view = ExtractedView {
        retained_view_entity: RetainedViewEntity::new(rendering.main_entity.into(), None, 0),
        clip_from_view: rendering.clip_from_view,
        world_from_view: rendering.world_from_view,
        clip_from_world: None,
        target_format: CORE_3D_DEPTH_FORMAT,
        viewport: UVec4::new(0, 0, size, size),
        color_grading: default(),
        invert_culling: false,
    };
    // Meshes have no render entity of their own; Bevy keys them by main entity.
    let casters = rendering
        .casters
        .iter()
        .map(|&caster| (Entity::PLACEHOLDER, MainEntity::from(caster)))
        .collect();
    commands.entity(render_entity).insert((
        view,
        RootNonCameraView(Core3d.intern()),
        NoIndirectDrawing,
        rendering.uniform,
        ShadowFaceView {
            target: rendering.target,
            casters,
        },
    ));
}

pub(super) fn extract_shadow_faces(
    mut commands: Commands,
    faces: Extract<Query<(Entity, &RenderEntity, &ShadowFace)>>,
    cascades: Extract<Query<(Entity, &RenderEntity, &SunCascadeView)>>,
    views: Query<Entity, With<ShadowFaceView>>,
    mut rendering: Local<EntityHashSet>,
) {
    rendering.clear();
    for (main_entity, render_entity, face) in &faces {
        if !face.renders {
            continue;
        }
        let render_entity = render_entity.id();
        rendering.insert(render_entity);
        let (world_from_view, clip_from_view) = face_view(face.pose, face.layer % CUBE_FACE_COUNT);
        let uniform = ShadowFaceUniform {
            radius: face.pose.radius,
            layer: face.layer,
            blur: face.pose.blur,
            orthographic: 0,
        };
        let face = Rendering {
            main_entity,
            world_from_view,
            clip_from_view,
            resolution: SHADOW_CUBEMAP_RESOLUTION,
            uniform,
            target: ShadowTarget::Face(face.layer),
            casters: &face.casters,
        };
        insert_view(&mut commands, render_entity, face);
    }
    for (main_entity, render_entity, cascade) in &cascades {
        let Some(pose) = cascade.pose.filter(|_| cascade.renders) else {
            continue;
        };
        let render_entity = render_entity.id();
        rendering.insert(render_entity);
        let (world_from_view, clip_from_view) = pose.view();
        let uniform = ShadowFaceUniform {
            radius: pose.range(),
            layer: cascade.index,
            blur: POINT_BLUR,
            orthographic: 1,
        };
        let cascade = Rendering {
            main_entity,
            world_from_view,
            clip_from_view,
            resolution: SUN_CASCADE_RESOLUTION,
            uniform,
            target: ShadowTarget::Cascade(cascade.index),
            casters: &cascade.casters,
        };
        insert_view(&mut commands, render_entity, cascade);
    }
    for view in &views {
        if !rendering.contains(&view) {
            commands.entity(view).remove::<FaceViewBundle>();
        }
    }
}
