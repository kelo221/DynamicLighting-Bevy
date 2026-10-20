//! Which probes need a capture, and the cube array and records built from the captured ones.

use core::cmp::Ordering;
use std::sync::Arc;

use bevy::asset::AssetEvent;
use bevy::ecs::entity::{EntityHashMap, EntityHashSet};
use bevy::prelude::*;

use super::assemble::{ProbeLighting, cube_array};
use super::capture::Stage;
use super::records::{ProbeBox, ProbeRecord, pack_probes};
use super::{CaptureReflectionProbes, REFLECTION_CUBES, ReflectionProbe};
use crate::baked_lighting::AppliedBake;
use crate::environment::SKY_SPECULAR;
use crate::environment::images::placeholder_specular;
use crate::shared::{
    LitShaderFeatures, MAX_REFLECTION_PROBES, ReflectionProbeRecords, ShaderFeaturesInUse,
};

/// A probe's capture.
pub(super) struct Captured {
    pub lighting: Arc<ProbeLighting>,
    pub capture: Vec3,
}

#[derive(Resource, Default)]
pub(super) struct Probes {
    /// Probes waiting for a capture.
    pub stale: EntityHashSet,
    pub results: EntityHashMap<Captured>,
    /// The captured probes in cube order: probe `i` is cube `i + 1`.
    order: Vec<Entity>,
    /// The cube array is out of date.
    pub rebuild: bool,
    pub stage: Stage,
    /// Frames in a row in which no render pipeline was compiling.
    pub idle_frames: u32,
    warned: bool,
}

/// Inserts the cube array of the placeholder sky, bound until there is a sky or a probe.
pub(super) fn insert_cube_array(app: &mut App) {
    let mut images = app.world_mut().resource_mut::<Assets<Image>>();
    images
        .insert(&REFLECTION_CUBES, cube_array(&placeholder_specular(), &[]))
        .expect("the reflection cube handle is a fixed UUID");
}

/// Probes whose settings or transform changed.
type ChangedProbes = (
    With<ReflectionProbe>,
    Or<(Changed<ReflectionProbe>, Changed<GlobalTransform>)>,
);

/// Marks probes stale when they change, and all of them when the scene's lighting does: a new
/// bake, sky or lit shader, or a [`CaptureReflectionProbes`] message.
#[allow(clippy::too_many_arguments)]
pub(super) fn mark_stale(
    changed: Query<Entity, ChangedProbes>,
    all: Query<Entity, With<ReflectionProbe>>,
    mut removed: RemovedComponents<ReflectionProbe>,
    mut requests: MessageReader<CaptureReflectionProbes>,
    mut images: MessageReader<AssetEvent<Image>>,
    bake: Option<Res<AppliedBake>>,
    features: Res<LitShaderFeatures>,
    mut probes: ResMut<Probes>,
) {
    let sky = images.read().any(|event| match event {
        AssetEvent::Added { id } | AssetEvent::Modified { id } => *id == SKY_SPECULAR.id(),
        _ => false,
    });
    let requested = requests.read().count() > 0;
    let rebaked = bake.is_some_and(|bake| bake.is_changed() && !bake.is_added());
    let probes = &mut *probes;
    probes.rebuild |= sky;
    for entity in removed.read() {
        probes.stale.remove(&entity);
        probes.rebuild |= probes.results.remove(&entity).is_some();
    }
    if sky || requested || rebaked || (features.is_changed() && !features.is_added()) {
        probes.stale.extend(&all);
    } else {
        probes.stale.extend(&changed);
    }
}

/// The probes' records for the light buffer, and the cube array when a capture or the sky
/// changed.
pub(super) fn update_probes(
    query: Query<(&ReflectionProbe, &GlobalTransform)>,
    mut probes: ResMut<Probes>,
    mut images: ResMut<Assets<Image>>,
    mut records: ResMut<ReflectionProbeRecords>,
    mut in_use: ResMut<ShaderFeaturesInUse>,
) {
    let probes = &mut *probes;
    if core::mem::take(&mut probes.rebuild) {
        let mut order: Vec<(Entity, f32, Vec3)> = probes
            .results
            .iter()
            .filter_map(|(&entity, captured)| {
                let (_, transform) = query.get(entity).ok()?;
                let bounds = ProbeBox::new(transform)?;
                Some((entity, bounds.volume(), captured.capture))
            })
            .collect();
        // Smallest box first, as the records; ties by capture point, never by entity.
        order.sort_by(|a, b| {
            a.1.total_cmp(&b.1).then_with(|| {
                a.2.to_array()
                    .partial_cmp(&b.2.to_array())
                    .unwrap_or(Ordering::Equal)
            })
        });
        if order.len() > MAX_REFLECTION_PROBES && !probes.warned {
            probes.warned = true;
            warn!(
                "dynlight reflection probes: {} probes, only the {MAX_REFLECTION_PROBES} smallest \
                 are used",
                order.len()
            );
        }
        order.truncate(MAX_REFLECTION_PROBES);
        probes.order = order.into_iter().map(|(entity, ..)| entity).collect();
        let cubes: Vec<&_> = probes
            .order
            .iter()
            .map(|entity| &probes.results[entity].lighting.specular)
            .collect();
        let fallback = placeholder_specular();
        let sky = images.get(&SKY_SPECULAR).unwrap_or(&fallback);
        let array = cube_array(sky, &cubes);
        let _ = images.insert(&REFLECTION_CUBES, array);
    }

    let list: Vec<ProbeRecord> = (probes.order.iter().enumerate())
        .filter_map(|(index, entity)| {
            let (probe, transform) = query.get(*entity).ok()?;
            let captured = probes.results.get(entity)?;
            Some(ProbeRecord {
                bounds: ProbeBox::new(transform)?,
                capture: captured.capture,
                layer: index as u32 + 1,
                irradiance: captured
                    .lighting
                    .irradiance
                    .map(|c| c * probe.diffuse_intensity.max(0.0)),
            })
        })
        .collect();
    let (packed, _) = pack_probes(&list);
    records.set_if_neq(ReflectionProbeRecords(packed));
    in_use.0.reflection_probes |= packed.count > 0;
}
