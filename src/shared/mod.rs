//! Small kernel used by two or more slices: stable bake identity, the GPU light record,
//! scene-wide settings, math helpers and the base WESL types module.
//!
//! Shaders import the types module as `dynlight::shared::shaders::types`.

mod bake_id;
pub mod cube;
mod globals;
mod gpu_light;
pub mod math;
pub mod mesh_hash;
mod settings;
#[cfg(test)]
pub(crate) mod wesl_check;

use bevy::prelude::*;
use bevy::shader::{Shader, load_shader_library};

pub use bake_id::{BakeId, ResolvedBakeId};
pub use globals::{GPU_LIGHT_GLOBALS_SIZE, GpuLightGlobals};
pub use gpu_light::{ChannelFlags, GPU_LIGHT_SIZE, GpuLight};
pub use mesh_hash::{hash_mesh, mesh_content_hash};
pub use settings::{
    DEBUG_LIGHT_COUNT_MAX, DebugView, DynamicGeometryMode, DynamicLightingSettings, RuntimeQuality,
    TrackingMode, TriangleIdSource,
};

pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DynamicLightingSettings>();
    }

    fn finish(&self, app: &mut App) {
        if shaders_available(app) {
            load_shader_library!(app, "shaders/types.wesl");
        }
    }
}

/// Whether the app can load shaders. Headless apps (servers, bake tools, tests) skip shader
/// loading; every slice that embeds WESL checks this in `Plugin::finish`.
pub fn shaders_available(app: &App) -> bool {
    app.world().contains_resource::<Assets<Shader>>()
}
