//! What changed for the shadow cubemaps and the sun's cascades since the last frame: shadow
//! casters that moved, appeared, disappeared, animate, changed render layers, mesh, material or
//! bake data, and the faces and cascades whose last rendering missed something (a caster's
//! pipeline still compiling, its mesh, material or textures still on their way to the GPU).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

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

/// What a shadow view renders into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShadowTarget {
    /// A layer of the shadow cubemap array.
    Face(u32),
    /// A layer of the sun's cascade array.
    Cascade(u32),
}

/// Filled by the render world: faces and cascades that rendered without some of their casters,
/// or all of them when the pass couldn't run at all.
#[derive(Resource, Clone, Default)]
pub(super) struct ShadowFeedback(Arc<Missed>);

#[derive(Default)]
struct Missed {
    every_face: AtomicBool,
    targets: Mutex<Vec<ShadowTarget>>,
}

impl ShadowFeedback {
    /// The pass couldn't run, so every face and cascade must render again.
    pub(super) fn report_incomplete(&self) {
        self.0.every_face.store(true, Ordering::Relaxed);
    }

    /// `target` rendered without some of its casters.
    pub(super) fn report(&self, target: ShadowTarget) {
        let mut targets = self
            .0
            .targets
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        targets.push(target);
    }

    /// Whether everything must render again, and the targets that must.
    fn take(&self) -> (bool, Vec<ShadowTarget>) {
        let every_face = self.0.every_face.swap(false, Ordering::Relaxed);
        let mut targets = self
            .0
            .targets
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        (every_face, core::mem::take(&mut *targets))
    }
}

/// The last known transform of every shadow caster, and its bounds while it is visible, so a
/// caster that moves or disappears is also reported where it was.
#[derive(Resource, Default)]
pub(super) struct CasterBounds(EntityHashMap<Seen>);

impl CasterBounds {
    /// The bounding spheres of the visible casters with bounds, leaving out baked ones when
    /// `moving_only`.
    pub(super) fn spheres(&self, moving_only: bool) -> Vec<(Vec3, f32)> {
        self.0
            .values()
            .filter_map(|seen| seen.bounds)
            .filter(|bounds| !(moving_only && bounds.baked))
            .map(|bounds| (bounds.center, bounds.radius))
            .collect()
    }
}

#[derive(Clone, Copy)]
struct Seen {
    transform: GlobalTransform,
    bounds: Option<CasterChange>,
}

/// Casters whose material or textures changed, reported by the material trackers in
/// `materials.rs` for [`track_casters`] to pick up.
#[derive(Resource, Default)]
pub(super) struct TouchedCasters(pub Vec<Entity>);

/// This frame's changes for the cube planning in `views.rs`.
#[derive(Resource, Default)]
pub(super) struct CasterChanges {
    pub changes: Vec<CasterChange>,
    /// Array layers whose last rendering missed some casters.
    pub redraw: Vec<u32>,
    /// The sun's cascades whose last rendering missed some casters, one bit each.
    pub redraw_cascades: u8,
    /// Frames left in which every face and cascade renders.
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
/// A caster that became baked is reported too, so its bounds say it is baked.
type Edited = (
    With<Mesh3d>,
    Or<(
        Changed<Mesh3d>,
        Changed<Aabb>,
        Changed<InheritedVisibility>,
        Changed<RenderLayers>,
        Added<BakedReceiver>,
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
pub(super) fn bounds(transform: &GlobalTransform, aabb: &Aabb, baked: bool) -> CasterChange {
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
        redraw,
        redraw_cascades,
        settle,
        lingering,
    } = &mut *changes;
    list.clear();
    let (every_face, missed) = feedback.take();
    redraw.clear();
    *redraw_cascades = 0;
    for target in missed {
        match target {
            ShadowTarget::Face(layer) => redraw.push(layer),
            ShadowTarget::Cascade(index) => *redraw_cascades |= 1 << index.min(7),
        }
    }
    // Which meshes are baked decides what cubes of baked lights draw.
    let unbaked: Vec<Entity> = unbaked.read().collect();
    if every_face | !rebaked.is_empty() | !unbaked.is_empty() {
        *settle = SETTLE_FRAMES;
    }

    for entity in removed.read() {
        list.extend(known.0.remove(&entity).and_then(|seen| seen.bounds));
    }
    let unbaked = unbaked
        .iter()
        .filter_map(|&entity| casters.get(entity).ok());
    for caster in edited.iter().chain(unbaked) {
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
        assert_eq!(feedback.take(), (false, vec![]));
        feedback.clone().report_incomplete();
        assert_eq!(feedback.take(), (true, vec![]));
        assert_eq!(feedback.take(), (false, vec![]));
        let render_world = feedback.clone();
        render_world.report(ShadowTarget::Face(7));
        render_world.report(ShadowTarget::Cascade(2));
        let missed = vec![ShadowTarget::Face(7), ShadowTarget::Cascade(2)];
        assert_eq!(feedback.take(), (false, missed));
        assert_eq!(feedback.take(), (false, vec![]));
    }
}
