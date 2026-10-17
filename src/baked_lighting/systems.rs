//! Uploads the applied bake and points baked meshes at their receiver records.

use bevy::asset::uuid_handle;
use bevy::mesh::MeshTag;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;

use super::layout::{BakedBuffers, flags as layout_flags, header};
use crate::shared::{BakeId, ResolvedBakeId, ShaderFeaturesInUse, hash_mesh};

/// Every baked mesh's triangle words (binding 23 of lit materials).
pub const BAKED_TRIANGLES: Handle<ShaderBuffer> =
    uuid_handle!("2b8e4c1d-7f3a-4e62-b0d9-5a1c6e8f3b27");
/// Receiver records, light BVH and distance cubes (binding 24 of lit materials).
pub const BAKED_AUX: Handle<ShaderBuffer> = uuid_handle!("9d4f2a6b-1e8c-4b3f-a7d5-0c6e9b2f4a18");

/// The bake currently applied to the scene. Replace [`Self::buffers`] to apply another bake;
/// the buffers are re-uploaded and receivers re-tagged on change.
#[derive(Resource, Debug)]
pub struct AppliedBake {
    pub buffers: BakedBuffers,
}

impl Default for AppliedBake {
    fn default() -> Self {
        Self {
            buffers: BakedBuffers::empty(),
        }
    }
}

/// Marks entities whose `MeshTag` was set by the baked-lighting slice, so it can be removed
/// again when the bake changes.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BakedReceiver;

/// Content hashes of mesh assets, computed once per asset version.
#[derive(Resource, Default)]
pub(super) struct MeshHashes {
    hashes: HashMap<AssetId<Mesh>, Option<u64>>,
    /// Bake ids already reported as stale, so the warning shows once per bake.
    reported: HashSet<BakeId>,
}

pub(super) fn create_buffers(app: &mut App) {
    let Some(mut assets) = app.world_mut().get_resource_mut::<Assets<ShaderBuffer>>() else {
        return;
    };
    let empty = BakedBuffers::empty();
    for (handle, words, label) in [
        (
            &BAKED_TRIANGLES,
            &empty.triangles,
            "dynlight baked triangles",
        ),
        (&BAKED_AUX, &empty.aux, "dynlight baked aux"),
    ] {
        let mut buffer = ShaderBuffer::from(words.clone());
        buffer.label = label.into();
        assets
            .insert(handle, buffer)
            .expect("the baked buffer handles are fixed UUIDs");
    }
}

/// Marks bounce light as used while the applied bake has bounce data.
pub(super) fn note_bounce(applied: Res<AppliedBake>, mut in_use: ResMut<ShaderFeaturesInUse>) {
    let flags = applied.buffers.aux.get(header::FLAGS).copied().unwrap_or(0);
    in_use.0.bounce |= flags & layout_flags::BOUNCE_LAYOUT != 0;
}

pub(super) fn upload_bake(applied: Res<AppliedBake>, assets: Option<ResMut<Assets<ShaderBuffer>>>) {
    if !applied.is_changed() {
        return;
    }
    let Some(mut assets) = assets else {
        return;
    };
    for (handle, words) in [
        (&BAKED_TRIANGLES, &applied.buffers.triangles),
        (&BAKED_AUX, &applied.buffers.aux),
    ] {
        if let Some(mut buffer) = assets.get_mut(handle) {
            buffer.clear();
            buffer.extend_from_slice(words.as_slice());
        }
    }
}

/// Forgets hashes of edited or removed meshes.
pub(super) fn track_mesh_edits(
    mut events: MessageReader<AssetEvent<Mesh>>,
    mut hashes: ResMut<MeshHashes>,
) {
    for event in events.read() {
        match event {
            AssetEvent::Modified { id }
            | AssetEvent::Removed { id }
            | AssetEvent::Unused { id } => {
                hashes.hashes.remove(id);
            }
            _ => {}
        }
    }
}

type ReceiverItem<'a> = (
    Entity,
    &'a ResolvedBakeId,
    &'a Mesh3d,
    Option<&'a MeshTag>,
    Has<BakedReceiver>,
);

/// Gives every baked mesh entity the `MeshTag` of its receiver record, when its mesh still
/// matches the bake, and takes tags away from meshes that no longer have a matching record.
pub(super) fn tag_receivers(
    mut commands: Commands,
    applied: Res<AppliedBake>,
    mut hashes: ResMut<MeshHashes>,
    meshes: Res<Assets<Mesh>>,
    receivers: Query<ReceiverItem>,
) {
    if applied.is_changed() {
        hashes.reported.clear();
    }
    let MeshHashes { hashes, reported } = &mut *hashes;
    for (entity, &ResolvedBakeId(id), mesh, tag, tagged) in &receivers {
        let wanted = applied.buffers.receivers.get(&id).and_then(|receiver| {
            let hash = *hashes
                .entry(mesh.id())
                .or_insert_with(|| meshes.get(mesh.id()).and_then(hash_mesh));
            match hash {
                Some(hash) if hash == receiver.content_hash => Some(receiver.tag),
                // Not loaded yet: the hash is recomputed once the mesh arrives.
                None if meshes.get(mesh.id()).is_none() => {
                    hashes.remove(&mesh.id());
                    None
                }
                _ => {
                    if reported.insert(id) {
                        warn!(
                            "mesh {:#018x} changed since it was baked; it is lit without baked \
                             shadows until the scene is baked again",
                            id.0
                        );
                    }
                    None
                }
            }
        });
        match wanted {
            Some(wanted) if tag.is_none_or(|tag| tag.value != wanted) || !tagged => {
                commands
                    .entity(entity)
                    .insert((MeshTag::new(wanted), BakedReceiver));
            }
            None if tagged => {
                commands.entity(entity).remove::<(MeshTag, BakedReceiver)>();
            }
            _ => {}
        }
    }
}
