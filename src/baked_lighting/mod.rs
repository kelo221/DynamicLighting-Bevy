//! Runtime side of the bake: uploads the triangle and aux buffers, points baked meshes at their
//! receiver records through `MeshTag`, and detects stale bakes. Shaders find a triangle's data
//! by primitive index, or by a triangle-ID vertex attribute on GPUs without primitive indices.
//!
//! Shader modules: `dynlight::baked_lighting::shaders::{baked_data, triangles}`.

mod apply;
mod bake_features;
mod features;
pub mod layout;
pub mod mirror;
#[cfg(test)]
pub(crate) mod shader_tests;
mod systems;
mod triangle_ids;

use bevy::prelude::*;
use bevy::render::{RenderApp, RenderStartup};
use bevy::shader::load_shader_library;

pub use apply::{ActiveBake, scene_buffers, scene_slots};
pub use bake_features::bake_features;
pub use features::primitive_index_supported;
pub use layout::{BakedBuffers, Receiver, ReceiverInput, SceneInput};
pub use systems::{AppliedBake, BAKED_AUX, BAKED_TRIANGLES, BakedReceiver};
pub use triangle_ids::{
    ATTRIBUTE_TRIANGLE_ID, TRIANGLE_ID_LOCATION, accepts_triangle_ids, add_triangle_ids,
    has_triangle_ids,
};

use crate::bake::{BakeSystems, DynLightBake};
use crate::light::LightSystems;
use crate::shared::shaders_available;

/// Baked-lighting work in `PostUpdate`, before the light buffer is packed.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BakedLightingSystems;

pub struct BakedLightingPlugin;

impl Plugin for BakedLightingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AppliedBake>()
            .init_resource::<ActiveBake>()
            .init_resource::<systems::MeshHashes>()
            .configure_sets(
                PostUpdate,
                BakedLightingSystems
                    .after(BakeSystems::ResolveIds)
                    .before(LightSystems::Locate)
                    .before(LightSystems::Attach),
            )
            .add_systems(
                PostUpdate,
                (
                    systems::upload_bake,
                    // Every frame, after a new bake is applied: the use is cleared in `Last`.
                    bake_features::note_bake_features.after(apply::apply_bake),
                )
                    .in_set(BakedLightingSystems),
            );
    }

    fn finish(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(RenderStartup, features::detect_features);
        }
        // Registered by the bake slice's `finish`, which runs first, when there is an asset server.
        if app.world().contains_resource::<Assets<DynLightBake>>() {
            app.add_systems(
                PostUpdate,
                (
                    apply::apply_bake
                        .before(systems::upload_bake)
                        .before(systems::tag_receivers),
                    apply::note_bake_loading,
                )
                    .in_set(BakedLightingSystems),
            );
        }
        // Receivers need mesh assets; headless bake tools have none.
        if app.world().contains_resource::<Assets<Mesh>>() {
            app.add_systems(
                PostUpdate,
                (
                    triangle_ids::attach_triangle_ids,
                    systems::track_mesh_edits,
                    systems::tag_receivers,
                )
                    .chain()
                    .in_set(BakedLightingSystems),
            );
        }
        if shaders_available(app) {
            load_shader_library!(app, "shaders/baked_data.wesl");
            load_shader_library!(app, "shaders/triangles.wesl");
            systems::create_buffers(app);
        }
    }
}
