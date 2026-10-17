//! Clustered realtime lights: a grid of cells over the view of the camera that chose the
//! realtime lights, each listing the realtime lights whose sphere reaches it, so a fragment only
//! visits those. Every light is still shaded the same way; only the set of lights a fragment
//! visits gets shorter. Built on the CPU each frame after the light buffer is packed.
//!
//! Shaders import `dynlight::light_clusters::shaders::clusters` (`realtime_span`, `span_light`).
//!
//! The slice also keeps Bevy's own light clusters, which serve only Bevy's lights, on the CPU in a
//! single cell (see `bevy_clusters`).

mod assign;
mod bevy_clusters;
mod buffer;
mod gpu;
mod grid;
#[cfg(test)]
pub(crate) mod shader_tests;
mod systems;
#[cfg(test)]
mod tests;

use bevy::prelude::*;
use bevy::shader::load_shader_library;

pub use assign::{ClusterLists, GridOff, MAX_ENTRIES, build, light_sphere};
pub use buffer::REALTIME_CLUSTERS;
pub use gpu::{GPU_CLUSTER_HEADER_SIZE, GpuClusterHeader};
pub use grid::{Cell, FIRST_SLICE_DEPTH, GRID, Grid, Sphere};
pub use systems::RealtimeClusters;

use crate::light::LightSystems;
use crate::shared::shaders_available;

/// The cluster lists are built and uploaded in `PostUpdate`, after the light buffer is packed.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LightClustersSystems;

pub struct LightClustersPlugin;

impl Plugin for LightClustersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RealtimeClusters>()
            .configure_sets(PostUpdate, LightClustersSystems.after(LightSystems::Pack))
            .add_systems(
                PostUpdate,
                (systems::build_clusters, systems::upload_clusters)
                    .chain()
                    .in_set(LightClustersSystems),
            )
            .add_systems(Startup, bevy_clusters::use_cpu_clustering)
            .add_systems(PreUpdate, bevy_clusters::single_cluster);
    }

    fn finish(&self, app: &mut App) {
        if shaders_available(app) {
            load_shader_library!(app, "shaders/clusters.wesl");
            buffer::create_cluster_buffer(app);
        }
    }
}
