//! The lit material ([`DynLitMaterial`]) and the WESL lighting composition that calls into the
//! other slices' shader modules.
//!
//! Shader modules: `dynlight::surface::shaders::{bindings, light_processor, lighting, dynlit}`;
//! `dynlit_vertex` is the vertex shader of meshes with triangle IDs.

mod alpha_mask;
mod material;
#[cfg(test)]
mod shader_tests;
mod shared_arrays;

use bevy::asset::{AssetPlugin, embedded_asset};
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use bevy::shader::load_shader_library;

pub use alpha_mask::{DerivedAlphaMask, material_alpha};
pub use material::{DynLitKey, DynLitMaterial, DynLitUniform, ShadingModel, flags};

use crate::realtime_shadows::CasterMaterialPlugin;
use crate::shared::shaders_available;

pub struct SurfacePlugin;

impl Plugin for SurfacePlugin {
    fn build(&self, app: &mut App) {
        // Materials need the asset server; headless apps (bake tools, servers, tests) skip them.
        // The entry shader must be registered before the material pipeline loads it.
        if app.is_plugin_added::<AssetPlugin>() {
            embedded_asset!(app, "shaders/dynlit.wesl");
            embedded_asset!(app, "shaders/dynlit_vertex.wesl");
            embedded_asset!(app, "shaders/dynlit_prepass.wesl");
            app.add_plugins((
                MaterialPlugin::<DynLitMaterial>::default(),
                CasterMaterialPlugin::<DynLitMaterial>::default(),
            ));
        }
    }

    fn finish(&self, app: &mut App) {
        let world = app.world();
        if world.contains_resource::<Assets<DynLitMaterial>>()
            && world.contains_resource::<Assets<Image>>()
        {
            // Before the bake snapshots the scene in `PostUpdate`.
            app.add_systems(
                Update,
                (
                    alpha_mask::sync_alpha_masks,
                    shared_arrays::rebind_shared_arrays,
                ),
            );
        }
        if shaders_available(app) {
            load_shader_library!(app, "shaders/bindings.wesl");
            load_shader_library!(app, "shaders/brdf.wesl");
            load_shader_library!(app, "shaders/light_processor.wesl");
            load_shader_library!(app, "shaders/lighting.wesl");
            load_shader_library!(app, "shaders/shade.wesl");
        }
    }
}
