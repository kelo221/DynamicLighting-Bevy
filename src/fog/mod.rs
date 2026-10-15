//! Volumetric fog around lights: the [`VolumetricLight`] component, the [`FogPostProcess`]
//! camera pass that draws it, packing of fog volumes for that pass, and a CPU reference of the
//! fog shader.
//!
//! Shader modules: `dynlight::fog::shaders::fog_volumes`; the pass entry is `fog.wesl`.

pub mod mirror;
mod params;
mod render;
#[cfg(test)]
mod shader_tests;
#[cfg(test)]
mod tests;
mod volumes;
mod volumetric;

use bevy::asset::{AssetPlugin, embedded_asset};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponentPlugin;
use bevy::render::render_resource::SpecializedRenderPipelines;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::load_shader_library;

pub use params::{Placement, bounding_radius, cone_term, is_active, pack, shaft_slope};
pub use volumes::{FOG_BUFFER, FogPostProcess, FogVolumes};
pub use volumetric::{VolumetricLight, VolumetricShape};

use crate::light::LightSystems;
use crate::shared::shaders_available;

pub struct FogPlugin;

impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<VolumetricLight>()
            .register_type::<FogPostProcess>()
            .init_resource::<FogVolumes>()
            .add_systems(
                PostUpdate,
                (volumes::pack_fog_volumes, volumes::upload_fog_volumes)
                    .chain()
                    .after(LightSystems::Pack),
            );
        // The pass needs the asset server and a renderer; headless apps only pack volumes.
        if !app.is_plugin_added::<AssetPlugin>() {
            return;
        }
        embedded_asset!(app, "shaders/fog.wesl");
        app.add_plugins(ExtractComponentPlugin::<FogPostProcess>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SpecializedRenderPipelines<render::FogPipeline>>()
            .add_systems(RenderStartup, render::init_fog_pipeline)
            .add_systems(
                Render,
                render::prepare_fog_pipelines.in_set(RenderSystems::Prepare),
            )
            .add_systems(
                Core3d,
                render::fog_pass
                    .in_set(Core3dSystems::PostProcess)
                    .after(tonemapping),
            );
    }

    fn finish(&self, app: &mut App) {
        if shaders_available(app) {
            load_shader_library!(app, "shaders/fog_volumes.wesl");
            volumes::create_fog_buffer(app);
        }
    }
}
