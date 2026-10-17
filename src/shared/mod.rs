//! Small kernel used by two or more slices: stable bake identity, the GPU light record,
//! scene-wide settings, math helpers, per-frame shader buffer writes, the base WESL types
//! module and the shader variant.
//!
//! Shaders import the types module as `dynlight::shared::shaders::types` and the generated
//! variant constants as `dynlight::shared::shaders::variant`.

mod bake_id;
pub mod cube;
mod feature_choice;
mod feature_use;
mod globals;
mod gpu_light;
pub mod math;
pub mod mesh_hash;
mod name_path;
mod settings;
mod shader_buffer;
mod variant;
#[cfg(test)]
pub(crate) mod wesl_check;

use bevy::prelude::*;
use bevy::shader::{Shader, load_shader_library};

pub use bake_id::{BakeId, ResolvedBakeId};
pub use feature_use::{LitShaderFeatures, SceneLoading, ShaderFeaturesInUse};
pub use globals::{GPU_LIGHT_GLOBALS_SIZE, GpuLightGlobals};
pub use gpu_light::{ChannelFlags, GPU_LIGHT_SIZE, GpuLight};
pub use mesh_hash::{hash_mesh, mesh_content_hash};
pub use name_path::NamePaths;
pub use settings::{
    DEBUG_LIGHT_COUNT_MAX, DebugView, DynamicGeometryMode, DynamicLightingSettings,
    FeatureSelection, RuntimeQuality, ShaderFeatures, TrackingMode, TriangleIdSource,
};
pub use shader_buffer::{buffer_capacity, frame_shader_buffer, write_shader_buffer};
pub use variant::{ShaderVariant, VARIANT_SHADER};

pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DynamicLightingSettings>();
        feature_use::plugin(app);
    }

    fn finish(&self, app: &mut App) {
        if shaders_available(app) {
            load_shader_library!(app, "shaders/types.wesl");
            variant::plugin(app);
        }
    }
}

/// Whether the app can load shaders. Headless apps (servers, bake tools, tests) skip shader
/// loading; every slice that embeds WESL checks this in `Plugin::finish`.
pub fn shaders_available(app: &App) -> bool {
    app.world().contains_resource::<Assets<Shader>>()
}
