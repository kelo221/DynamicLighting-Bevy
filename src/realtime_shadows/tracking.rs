//! What changed for the shadow cubemaps since the last frame: shadow casters that moved,
//! appeared, disappeared, animate, changed render layers, mesh or material, and anything that
//! may have kept the last renderings from being complete (pipelines still compiling, new meshes,
//! materials and textures still on their way to the GPU).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bevy::asset::asset_changed::AssetChanged;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
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

/// The last known transform of every shadow caster, and its bounds while it is visible, so a
/// caster that moves or disappears is also reported where it was.
#[derive(Resource, Default)]
pub(super) struct CasterBounds(EntityHashMap<Seen>);

#[derive(Clone, Copy)]
struct Seen {
    transform: GlobalTransform,
    bounds: Option<CasterChange>,
}

/// Casters whose material or textures changed, reported by the material trackers in
/// `materials.rs` for [`track_casters`] to pick up.
#[derive(Resource, Default)]
pub(super) struct TouchedCasters(pub Vec<Entity>);

/// This frame's changes for the cube planning in `cameras.rs`.
#[derive(Resource, Default)]
pub(super) struct CasterChanges {
    pub changes: Vec<CasterChange>,
    /// Frames left in which every face renders.
    pub settle: u32,
    /// Casters with a new mesh, material or textures, and the frames left in which they are
    /// reported where they are: the new data may take a frame or two to reach the GPU.
    lingering: EntityHashMap<u32>,
}

type CasterItem<'a> = (
    Entity,
    &'a GlobalTransform,
    Option<&'a Aabb>,
    &'a InheritedVisibility,
    Has<BakedReceiver>,
);
type Edited = (
    With<Mesh3d>,
    Or<(
        Changed<Mesh3d>,
        Changed<Aabb>,
        Changed<InheritedVisibility>,
        Changed<RenderLayers>,
    )>,
);
/// Systems that set a transform every frame mark it changed even while it stays put, so these
/// casters only count when their transform differs from the last one seen.
type Moved = (With<Mesh3d>, Changed<GlobalTransform>);
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

type Caster<'a> = (
    Entity,
    &'a GlobalTransform,
    Option<&'a Aabb>,
    &'a InheritedVisibility,
    bool,
);

/// Where a visible caster is now. Without bounds (`NoFrustumCulling`) it could be anywhere, so
/// every face renders.
fn current(caster: Caster, settle: &mut u32) -> Option<CasterChange> {
    let (_, transform, aabb, visibility, baked) = caster;
    if !visibility.get() {
        return None;
    }
    let Some(aabb) = aabb else {
        *settle = (*settle).max(1);
        return None;
    };
    Some(bounds(transform, aabb, baked))
}

/// Reports a caster where it was and where it is now, and remembers where it is.
fn report(
    caster: Caster,
    known: &mut CasterBounds,
    list: &mut Vec<CasterChange>,
    settle: &mut u32,
) {
    let (entity, transform, ..) = caster;
    list.extend(known.0.get(&entity).and_then(|seen| seen.bounds));
    let bounds = current(caster, settle);
    list.extend(bounds);
    let transform = *transform;
    known.0.insert(entity, Seen { transform, bounds });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn track_casters(
    edited: Query<CasterItem, Edited>,
    moved: Query<CasterItem, Moved>,
    animated: Query<CasterItem, Animated>,
    casters: Query<CasterItem, With<Mesh3d>>,
    remeshed: Query<Entity, (With<Mesh3d>, AssetChanged<Mesh3d>)>,
    rebaked: Query<(), (With<Mesh3d>, Added<BakedReceiver>)>,
    mut unbaked: RemovedComponents<BakedReceiver>,
    mut relayered: RemovedComponents<RenderLayers>,
    mut removed: RemovedComponents<Mesh3d>,
    mut touched: ResMut<TouchedCasters>,
    feedback: Res<ShadowFeedback>,
    mut known: ResMut<CasterBounds>,
    mut changes: ResMut<CasterChanges>,
) {
    let CasterChanges {
        changes: list,
        settle,
        lingering,
    } = &mut *changes;
    list.clear();
    // Which meshes are baked decides what cubes of baked lights draw.
    if feedback.take() | !rebaked.is_empty() | (unbaked.read().count() > 0) {
        *settle = SETTLE_FRAMES;
    }

    for entity in removed.read() {
        list.extend(known.0.remove(&entity).and_then(|seen| seen.bounds));
    }
    for caster in &edited {
        report(caster, &mut known, list, settle);
    }
    for caster in &moved {
        let (entity, transform, ..) = caster;
        let seen = known.0.get(&entity).map(|seen| seen.transform);
        if seen != Some(*transform) {
            report(caster, &mut known, list, settle);
        }
    }
    for caster in &animated {
        list.extend(current(caster, settle));
    }

    let fresh = touched.0.drain(..).chain(&remeshed).chain(relayered.read());
    for entity in fresh {
        lingering.insert(entity, SETTLE_FRAMES);
    }
    lingering.retain(|&entity, frames| {
        let Ok(caster) = casters.get(entity) else {
            return false;
        };
        list.extend(current(caster, settle));
        *frames -= 1;
        *frames > 0
    });
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
