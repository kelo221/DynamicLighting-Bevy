//! Uploads the applied bake and points baked meshes at their receiver records.

use bevy::asset::{RenderAssetUsages, uuid_handle};
use bevy::mesh::MeshTag;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;

use super::layout::BakedBuffers;
use crate::bake::{BlendedSurface, SurfacePending};
use crate::shared::{BakeId, NoShadowCasting, ResolvedBakeId, hash_mesh};

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
    /// Bake ids already reported for casting shadows differently than in the bake.
    reported_shadows: HashSet<BakeId>,
}

/// A bake buffer holding `words`, kept only in the render world: Bevy takes the CPU data when it
/// uploads the buffer, so the main world doesn't hold a second copy of a big bake.
fn bake_buffer(words: &[u32], label: &'static str) -> ShaderBuffer {
    let mut buffer = ShaderBuffer::new(words.to_vec(), RenderAssetUsages::RENDER_WORLD);
    buffer.label = label.into();
    buffer
}

const BUFFERS: [(&Handle<ShaderBuffer>, &str); 2] = [
    (&BAKED_TRIANGLES, "dynlight baked triangles"),
    (&BAKED_AUX, "dynlight baked aux"),
];

pub(super) fn create_buffers(app: &mut App) {
    let Some(mut assets) = app.world_mut().get_resource_mut::<Assets<ShaderBuffer>>() else {
        return;
    };
    let empty = BakedBuffers::empty();
    for ((handle, label), words) in BUFFERS.into_iter().zip([&empty.triangles, &empty.aux]) {
        assets
            .insert(handle, bake_buffer(words, label))
            .expect("the baked buffer handles are fixed UUIDs");
    }
}

/// Replaces both buffers when the applied bake changes. The assets are replaced, not edited:
/// once uploaded they have no CPU data left to extend.
pub(super) fn upload_bake(applied: Res<AppliedBake>, assets: Option<ResMut<Assets<ShaderBuffer>>>) {
    if !applied.is_changed() {
        return;
    }
    let Some(mut assets) = assets else {
        return;
    };
    let words = [&applied.buffers.triangles, &applied.buffers.aux];
    for ((handle, label), words) in BUFFERS.into_iter().zip(words) {
        if let Err(error) = assets.insert(handle, bake_buffer(words, label)) {
            error!("couldn't upload {label}: {error}");
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
    Has<NoShadowCasting>,
    Has<BlendedSurface>,
    Has<SurfacePending>,
);

/// Gives every baked mesh entity the `MeshTag` of its receiver record, when its mesh still
/// matches the bake, and takes tags away from meshes that no longer have a matching record. A
/// mesh that now casts shadows differently keeps its record (its own lighting still matches),
/// but the shadows it casts on others are stale, which is reported once its material is known.
pub(super) fn tag_receivers(
    mut commands: Commands,
    applied: Res<AppliedBake>,
    mut hashes: ResMut<MeshHashes>,
    meshes: Res<Assets<Mesh>>,
    receivers: Query<ReceiverItem>,
) {
    if applied.is_changed() {
        hashes.reported.clear();
        hashes.reported_shadows.clear();
    }
    let MeshHashes {
        hashes,
        reported,
        reported_shadows,
    } = &mut *hashes;
    for (entity, &ResolvedBakeId(id), mesh, tag, tagged, no_shadows, blended, pending) in &receivers
    {
        let wanted = applied.buffers.receivers.get(&id).and_then(|receiver| {
            let hash = *hashes
                .entry(mesh.id())
                .or_insert_with(|| meshes.get(mesh.id()).and_then(hash_mesh));
            match hash {
                Some(hash) if hash == receiver.content_hash => {
                    if !pending
                        && receiver.occludes == (no_shadows || blended)
                        && reported_shadows.insert(id)
                    {
                        warn!(
                            "mesh {:#018x} {} shadows since it was baked; the shadows it casts \
                             are out of date until the scene is baked again",
                            id.0,
                            if receiver.occludes {
                                "no longer casts"
                            } else {
                                "now casts"
                            }
                        );
                    }
                    Some(receiver.tag)
                }
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::asset::AssetPlugin;
    use bevy::render::storage::ShaderBufferData;

    use super::*;

    fn buffers(words: &[u32]) -> BakedBuffers {
        BakedBuffers {
            triangles: Arc::new(words.to_vec()),
            aux: Arc::new(words.iter().rev().copied().collect()),
            ..default()
        }
    }

    /// What Bevy does to a `RENDER_WORLD`-only buffer when it uploads it.
    fn extract(assets: &mut Assets<ShaderBuffer>, handle: &Handle<ShaderBuffer>) {
        let mut buffer = assets.get_mut(handle).unwrap();
        let len = buffer.buffer_size();
        buffer.data = ShaderBufferData::Uninitialized(len);
    }

    #[test]
    fn a_bake_can_be_applied_again_after_the_buffers_were_uploaded() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<ShaderBuffer>()
            .init_resource::<AppliedBake>()
            .add_systems(Update, upload_bake);
        for words in [&[1, 2, 3][..], &[4, 5], &[4, 5]] {
            app.world_mut().resource_mut::<AppliedBake>().buffers = buffers(words);
            app.update();
            let mut assets = app.world_mut().resource_mut::<Assets<ShaderBuffer>>();
            for (handle, expected) in [
                (&BAKED_TRIANGLES, words.to_vec()),
                (&BAKED_AUX, words.iter().rev().copied().collect()),
            ] {
                let buffer = assets.get(handle).unwrap();
                assert_eq!(buffer.asset_usage, RenderAssetUsages::RENDER_WORLD);
                assert_eq!(buffer.cast_slice::<u32>().unwrap(), expected);
                extract(&mut assets, handle);
            }
        }
    }
}
