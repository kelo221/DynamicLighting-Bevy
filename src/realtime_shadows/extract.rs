//! Hands every face rendering this frame to the render world as a view of its own, run through
//! the `Core3d` schedule before the cameras like Bevy's point-light shadow views, with the
//! casters it draws. Faces that don't render lose their view, so nothing runs for them.

use bevy::core_pipeline::Core3d;
use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::core_pipeline::schedule::RootNonCameraView;
use bevy::ecs::entity::EntityHashSet;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
use bevy::render::Extract;
use bevy::render::sync_world::{MainEntity, RenderEntity};
use bevy::render::view::{ExtractedView, NoIndirectDrawing, RetainedViewEntity};

use super::faces::SHADOW_CUBEMAP_RESOLUTION;
use super::views::{ShadowFace, ShadowFaceUniform, face_view};
use crate::shared::cube::CUBE_FACE_COUNT;

/// A face rendering this frame, in the render world: its array layer and the casters it draws.
#[derive(Component)]
pub(super) struct ShadowFaceView {
    pub layer: u32,
    pub casters: Vec<(Entity, MainEntity)>,
}

type FaceViewBundle = (
    ExtractedView,
    RootNonCameraView,
    NoIndirectDrawing,
    ShadowFaceUniform,
    ShadowFaceView,
);

pub(super) fn extract_shadow_faces(
    mut commands: Commands,
    faces: Extract<Query<(Entity, &RenderEntity, &ShadowFace)>>,
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
        let view = ExtractedView {
            retained_view_entity: RetainedViewEntity::new(main_entity.into(), None, 0),
            clip_from_view,
            world_from_view,
            clip_from_world: None,
            target_format: CORE_3D_DEPTH_FORMAT,
            viewport: UVec4::new(0, 0, SHADOW_CUBEMAP_RESOLUTION, SHADOW_CUBEMAP_RESOLUTION),
            color_grading: default(),
            invert_culling: false,
        };
        // Meshes have no render entity of their own; Bevy keys them by main entity.
        let face_casters = face
            .casters
            .iter()
            .map(|&caster| (Entity::PLACEHOLDER, MainEntity::from(caster)))
            .collect();
        commands.entity(render_entity).insert((
            view,
            RootNonCameraView(Core3d.intern()),
            NoIndirectDrawing,
            ShadowFaceUniform {
                radius: face.pose.radius,
                layer: face.layer,
                blur: face.pose.blur,
            },
            ShadowFaceView {
                layer: face.layer,
                casters: face_casters,
            },
        ));
    }
    for view in &views {
        if !rendering.contains(&view) {
            commands.entity(view).remove::<FaceViewBundle>();
        }
    }
}
