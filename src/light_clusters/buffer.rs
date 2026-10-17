//! The realtime cluster buffer asset every lit material binds.

use bevy::asset::uuid_handle;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;

use super::assign::ClusterLists;
use crate::shared::frame_shader_buffer;

/// The cluster buffer every `DynLitMaterial` binds at 29. Its layout is the WESL
/// `RealtimeClusters`: the [`GpuClusterHeader`](super::GpuClusterHeader), then the words.
pub const REALTIME_CLUSTERS: Handle<ShaderBuffer> =
    uuid_handle!("8e2d7c41-6b0a-4f3e-b915-3a4c5d6e7f80");

/// Creates the buffer asset with the grid off; a no-op without a renderer.
pub(super) fn create_cluster_buffer(app: &mut App) {
    let Some(mut buffers) = app.world_mut().get_resource_mut::<Assets<ShaderBuffer>>() else {
        return;
    };
    let buffer = frame_shader_buffer(
        &ClusterLists::default().to_bytes(),
        "dynlight realtime clusters",
    );
    buffers
        .insert(&REALTIME_CLUSTERS, buffer)
        .expect("the cluster buffer handle is a fixed UUID");
}
