//! What changed for the shadow cubemaps since the last frame: shadow casters that moved,
//! appeared, disappeared or animate, and anything that may have kept the last renderings from
//! being complete (pipelines still compiling, meshes and textures still arriving).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::camera::primitives::Aabb;
use bevy::ecs::entity::EntityHashMap;
use bevy::mesh::morph::MeshMorphWeights;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;

use super::slots::CasterChange;
use crate::baked_lighting::BakedReceiver;

/// Frames in which every face renders after something may have been missing. Rendering runs a
/// frame behind, so a report arrives a frame or two late.
const SETTLE_FRAMES: u32 = 4;

/// Set by the render world when a face pass ran while a pipeline was still compiling (so some
/// casters weren't drawn) or couldn't run at all.
#[derive(Resource, Clone, Default)]
pub(super) struct ShadowFeedback(Arc<AtomicBool>);

impl ShadowFeedback {
    pub(super) fn report_incomplete(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

/// The last known bounds of every shadow caster, so a caster that moves or disappears is also
/// reported where it was.
#[derive(Resource, Default)]
pub(super) struct CasterBounds(EntityHashMap<CasterChange>);

/// This frame's changes for the cube planning in `cameras.rs`.
#[derive(Resource, Default)]
pub(super) struct CasterChanges {
    pub changes: Vec<CasterChange>,
    /// Frames left in which every face renders.
    pub settle: u32,
}

type CasterItem<'a> = (
    Entity,
    &'a GlobalTransform,
    Option<&'a Aabb>,
    &'a InheritedVisibility,
    Has<BakedReceiver>,
);
type Changed3d = (
    With<Mesh3d>,
    Or<(
        Changed<GlobalTransform>,
        Changed<Mesh3d>,
        Changed<Aabb>,
        Changed<InheritedVisibility>,
    )>,
);
/// Skinned meshes move with their joints and morph targets change shape, both without a
/// transform change of their own.
type Animated = (
    With<Mesh3d>,
    Or<(With<SkinnedMesh>, Changed<MeshMorphWeights>)>,
);

/// The world-space bounding sphere of a caster.
fn bounds(transform: &GlobalTransform, aabb: &Aabb, baked: bool) -> CasterChange {
    let matrix = transform.affine().matrix3;
    let scale = matrix
        .x_axis
        .length()
        .max(matrix.y_axis.length())
        .max(matrix.z_axis.length());
    CasterChange {
        center: transform.transform_point(aabb.center.into()),
        radius: aabb.half_extents.length() * scale,
        baked,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn track_casters(
    changed: Query<CasterItem, Changed3d>,
    animated: Query<CasterItem, Animated>,
    rebaked: Query<(), (With<Mesh3d>, Added<BakedReceiver>)>,
    mut unbaked: RemovedComponents<BakedReceiver>,
    mut removed: RemovedComponents<Mesh3d>,
    mut meshes: MessageReader<AssetEvent<Mesh>>,
    mut images: MessageReader<AssetEvent<Image>>,
    feedback: Res<ShadowFeedback>,
    mut known: ResMut<CasterBounds>,
    mut changes: ResMut<CasterChanges>,
) {
    let changes = &mut *changes;
    changes.changes.clear();
    // Which meshes are baked decides what cubes of baked lights draw; new meshes and textures
    // may still be on their way to the GPU.
    let events = unbaked.read().count() + meshes.read().count() + images.read().count();
    let unsettled = feedback.take() | !rebaked.is_empty() | (events > 0);
    if unsettled {
        changes.settle = SETTLE_FRAMES;
    }

    for entity in removed.read() {
        if let Some(old) = known.0.remove(&entity) {
            changes.changes.push(old);
        }
    }
    for (entity, transform, aabb, visibility, baked) in &changed {
        if let Some(old) = known.0.remove(&entity) {
            changes.changes.push(old);
        }
        if !visibility.get() {
            continue;
        }
        let Some(aabb) = aabb else {
            // Without bounds (`NoFrustumCulling`) it could be anywhere.
            changes.settle = changes.settle.max(1);
            continue;
        };
        let new = bounds(transform, aabb, baked);
        known.0.insert(entity, new);
        changes.changes.push(new);
    }
    for (_, transform, aabb, visibility, baked) in &animated {
        match aabb {
            Some(aabb) if visibility.get() => changes.changes.push(bounds(transform, aabb, baked)),
            Some(_) => {}
            None => changes.settle = changes.settle.max(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_follow_translation_rotation_and_scale() {
        let aabb = Aabb::from_min_max(Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0));
        let transform = GlobalTransform::from(
            Transform::from_xyz(5.0, 0.0, 0.0)
                .with_rotation(Quat::from_rotation_z(core::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::new(1.0, 2.0, 1.0)),
        );
        let sphere = bounds(&transform, &aabb, true);
        assert!(sphere.center.abs_diff_eq(Vec3::new(3.0, 0.0, 0.0), 1e-5));
        assert!((sphere.radius - 3f32.sqrt() * 2.0).abs() < 1e-5);
        assert!(sphere.baked);
    }

    #[test]
    fn feedback_is_taken_once() {
        let feedback = ShadowFeedback::default();
        assert!(!feedback.take());
        feedback.clone().report_incomplete();
        assert!(feedback.take());
        assert!(!feedback.take());
    }
}
