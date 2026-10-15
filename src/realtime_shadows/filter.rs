//! Keeps baked meshes out of the cubemaps of lights that use their baked shadows: shadows of
//! baked geometry are already in the bake, as shadow bits on baked meshes and as distance cubes
//! for moving ones. Those cubemaps only need what the bake can't know about.

use core::any::TypeId;

use bevy::camera::visibility::VisibleEntities;
use bevy::prelude::*;

use super::cameras::ShadowFaceCamera;
use crate::baked_lighting::BakedReceiver;

/// Removes baked meshes from what the face cameras of such lights draw, after Bevy has
/// computed each view's visible entities.
pub(super) fn drop_baked_casters(
    mut faces: Query<(&ShadowFaceCamera, &Camera, &mut VisibleEntities)>,
    baked: Query<(), With<BakedReceiver>>,
) {
    for (face, camera, mut visible) in &mut faces {
        if camera.is_active && face.moving_only {
            visible
                .get_mut(TypeId::of::<Mesh3d>())
                .retain(|&entity| !baked.contains(entity));
        }
    }
}
