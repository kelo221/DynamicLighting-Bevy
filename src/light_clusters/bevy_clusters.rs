//! Bevy's own light clusters, kept as cheap as possible. They only list Bevy's lights, which
//! dynlight never uses, but every material draw binds Bevy's mesh view bind group, and Bevy only
//! builds that for views with cluster bindings. So cameras keep their `Clusters`, but Bevy clusters
//! them on the CPU, in a single cell: GPU clustering would run its compute passes for every camera
//! every frame with nothing to sort.

use bevy::light::cluster::{ClusterConfig, GlobalClusterSettings};
use bevy::prelude::*;

/// Switches Bevy to CPU clustering. Runs at startup, after every plugin has finished, because
/// Bevy inserts [`GlobalClusterSettings`] when its renderer finishes.
pub(super) fn use_cpu_clustering(settings: Option<ResMut<GlobalClusterSettings>>) {
    if let Some(mut settings) = settings
        && settings.gpu_clustering.is_some()
    {
        settings.gpu_clustering = None;
    }
}

/// Gives cameras without a cluster configuration a single cluster.
pub(super) fn single_cluster(
    mut commands: Commands,
    cameras: Query<Entity, (With<Camera3d>, Without<ClusterConfig>)>,
) {
    for camera in &cameras {
        commands.entity(camera).insert(ClusterConfig::Single);
    }
}
