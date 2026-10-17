//! Bake identity: which [`BakeId`] each baked mesh and light gets. Explicit ids are kept; the
//! rest are derived from the `Name` path (plus the mesh asset path for meshes). The bake and the
//! runtime both use this, so loaded bake data finds its entities again.

use std::collections::BTreeMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::light::DynamicLight;
use crate::shared::{BakeId, NamePaths, ResolvedBakeId, hash_mesh};

use super::settings::BakeStatic;

/// A mesh or light awaiting its id.
pub(super) struct Candidate {
    pub(super) explicit: Option<BakeId>,
    pub(super) segments: Vec<String>,
    /// Orders entities whose derived ids collide: content hash, then position bits.
    pub(super) tiebreak: (u64, [u32; 3]),
}

impl Candidate {
    pub(super) fn mesh(
        paths: &NamePaths,
        entity: Entity,
        mesh: &Handle<Mesh>,
        explicit: Option<BakeId>,
        content_hash: u64,
        position: Vec3,
    ) -> Self {
        let mut segments = paths.name_path(entity);
        if let Some(path) = mesh.path() {
            segments.push(format!("mesh {path}"));
        }
        Self {
            explicit,
            segments,
            tiebreak: (content_hash, bits(position)),
        }
    }

    pub(super) fn light(
        paths: &NamePaths,
        entity: Entity,
        explicit: Option<BakeId>,
        position: Vec3,
    ) -> Self {
        Self {
            explicit,
            segments: paths.name_path(entity),
            tiebreak: (0, bits(position)),
        }
    }
}

fn bits(position: Vec3) -> [u32; 3] {
    position.to_array().map(f32::to_bits)
}

/// Explicit ids are kept; derived ids that collide get a `#k` segment, in tiebreak order.
pub(super) fn assign_ids(
    candidates: &[Candidate],
    kind: &str,
    warn_collisions: bool,
) -> Vec<BakeId> {
    let derive = |segments: &[String]| BakeId::from_path(segments.iter().map(String::as_str));
    let mut ids: Vec<BakeId> = candidates
        .iter()
        .map(|c| c.explicit.unwrap_or_else(|| derive(&c.segments)))
        .collect();
    let mut groups: BTreeMap<BakeId, Vec<usize>> = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if candidate.explicit.is_none() {
            groups.entry(ids[index]).or_default().push(index);
        }
    }
    for (id, mut group) in groups.into_iter().filter(|(_, group)| group.len() > 1) {
        if warn_collisions {
            warn!(
                "dynlight bake: {} {kind}s derive the same BakeId {:#018x}; give them unique \
                 Names or explicit BakeIds so the bake keeps matching them",
                group.len(),
                id.0
            );
        }
        group.sort_by_key(|&index| candidates[index].tiebreak);
        for (k, &index) in group.iter().enumerate().skip(1) {
            let mut segments = candidates[index].segments.clone();
            segments.push(format!("#{k}"));
            ids[index] = derive(&segments);
        }
    }
    ids
}

type MeshItem<'a> = (
    Entity,
    &'a Mesh3d,
    &'a GlobalTransform,
    Option<&'a BakeId>,
    Option<&'a ResolvedBakeId>,
);
type LightItem<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    Option<&'a BakeId>,
    Option<&'a ResolvedBakeId>,
);
type IdChanges = Or<(
    Added<BakeStatic>,
    Added<DynamicLight>,
    Changed<Name>,
    Changed<ChildOf>,
    Changed<BakeId>,
    Changed<Mesh3d>,
)>;

/// Entities that still carry an id but are neither baked meshes nor lights.
type Unclaimed = (
    With<ResolvedBakeId>,
    Without<BakeStatic>,
    Without<DynamicLight>,
);

/// What can change an id since the last run.
#[derive(SystemParam)]
pub(super) struct IdTriggers<'w, 's> {
    changed: Query<'w, 's, (), IdChanges>,
    lights_changed: Query<'w, 's, (), Changed<DynamicLight>>,
    /// Absent in headless apps without mesh assets.
    mesh_events: Option<MessageReader<'w, 's, AssetEvent<Mesh>>>,
    removed_static: RemovedComponents<'w, 's, BakeStatic>,
    removed_lights: RemovedComponents<'w, 's, DynamicLight>,
    removed_ids: RemovedComponents<'w, 's, BakeId>,
    removed_names: RemovedComponents<'w, 's, Name>,
}

impl IdTriggers<'_, '_> {
    /// Whether meshes and lights need their ids resolved again. Drains the change streams.
    fn take(&mut self) -> (bool, bool) {
        let structure = !self.changed.is_empty()
            | (self.removed_static.read().count() > 0)
            | (self.removed_lights.read().count() > 0)
            | (self.removed_ids.read().count() > 0)
            | (self.removed_names.read().count() > 0);
        let meshes = self
            .mesh_events
            .as_mut()
            .is_some_and(|events| events.read().count() > 0);
        // Toggling `realtime` adds or removes a light from the bake.
        let lights = !self.lights_changed.is_empty();
        (structure || meshes, structure || lights)
    }
}

/// Keeps [`ResolvedBakeId`] on every [`BakeStatic`] mesh and baked light current.
pub(super) fn resolve_bake_ids(
    mut commands: Commands,
    mut triggers: IdTriggers,
    paths: NamePaths,
    meshes: Query<MeshItem, With<BakeStatic>>,
    lights: Query<LightItem>,
    stale: Query<Entity, Unclaimed>,
    mesh_assets: Option<Res<Assets<Mesh>>>,
) {
    let (resolve_meshes, resolve_lights) = triggers.take();
    let mut apply = |entity: Entity, id: BakeId, current: Option<&ResolvedBakeId>| {
        if current != Some(&ResolvedBakeId(id)) {
            commands.entity(entity).insert(ResolvedBakeId(id));
        }
    };
    if resolve_meshes {
        let candidates: Vec<Candidate> = meshes
            .iter()
            .map(|(entity, mesh, transform, explicit, _)| {
                let hash = mesh_assets
                    .as_deref()
                    .and_then(|assets| assets.get(&mesh.0))
                    .and_then(hash_mesh)
                    .unwrap_or(0);
                let position = transform.translation();
                Candidate::mesh(&paths, entity, &mesh.0, explicit.copied(), hash, position)
            })
            .collect();
        for ((entity, .., current), id) in meshes.iter().zip(assign_ids(&candidates, "mesh", false))
        {
            apply(entity, id, current);
        }
    }
    if resolve_lights {
        let baked = || lights.iter().filter(|(_, light, ..)| !light.realtime);
        let candidates: Vec<Candidate> = baked()
            .map(|(entity, _, transform, explicit, _)| {
                Candidate::light(&paths, entity, explicit.copied(), transform.translation())
            })
            .collect();
        for ((entity, .., current), id) in baked().zip(assign_ids(&candidates, "light", false)) {
            apply(entity, id, current);
        }
        for (entity, light, .., current) in &lights {
            if light.realtime && current.is_some() {
                commands.entity(entity).remove::<ResolvedBakeId>();
            }
        }
    }
    for entity in &stale {
        commands.entity(entity).remove::<ResolvedBakeId>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(explicit: Option<u64>, name: &str, x: f32) -> Candidate {
        Candidate {
            explicit: explicit.map(BakeId),
            segments: vec![name.to_owned()],
            tiebreak: (0, bits(Vec3::X * x)),
        }
    }

    #[test]
    fn derived_ids_are_disambiguated_in_tiebreak_order() {
        let ids = assign_ids(
            &[
                candidate(None, "Lamp", 2.0),
                candidate(Some(7), "Lamp", 0.0),
                candidate(None, "Lamp", 1.0),
                candidate(None, "Desk", 0.0),
            ],
            "light",
            false,
        );
        let lamp = BakeId::from_path(["Lamp"]);
        assert_eq!(ids[1], BakeId(7));
        assert_eq!(
            ids[2], lamp,
            "the first in tiebreak order keeps the plain id"
        );
        assert_eq!(ids[0], BakeId::from_path(["Lamp", "#1"]));
        assert_eq!(ids[3], BakeId::from_path(["Desk"]));
    }
}
