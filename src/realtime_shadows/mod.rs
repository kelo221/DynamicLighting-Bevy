//! Realtime variance shadow cubemaps for lights with `ShadowMode::Realtime`: six depth-only
//! cameras per light, turned into blurred distance moments by our own `Core3d` systems and
//! sampled by the lit material.
//!
//! Shader modules: `dynlight::realtime_shadows::shaders::{vsm, shadow_kernel}`; the face pass
//! entries are `shadow_moments.wesl` and `shadow_blur.wesl`.

mod cameras;
mod cubemaps;
pub mod faces;
mod render;
#[cfg(test)]
pub(crate) mod shader_tests;

use bevy::asset::{AssetPlugin, embedded_asset};
use bevy::camera::visibility::VisibilitySystems;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponentPlugin, UniformComponentPlugin};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::load_shader_library;
use bevy::transform::TransformSystems;

pub use cameras::{ShadowFaceCamera, ShadowFaceUniform};
pub use cubemaps::{
    SHADOW_CUBEMAP_FORMATS, SHADOW_CUBEMAPS, ShadowCubemapCapacity, cubemap_array,
    shadow_cubemap_format,
};

use crate::shared::shaders_available;

pub struct RealtimeShadowsPlugin;

impl Plugin for RealtimeShadowsPlugin {
    fn build(&self, app: &mut App) {
        // Face cameras and the cube array need a renderer; headless apps skip realtime shadows.
        if !app.is_plugin_added::<AssetPlugin>() {
            return;
        }
        embedded_asset!(app, "shaders/shadow_moments.wesl");
        embedded_asset!(app, "shaders/shadow_blur.wesl");
        app.init_resource::<cameras::ShadowCameras>()
            .init_resource::<ShadowCubemapCapacity>()
            .add_plugins((
                ExtractComponentPlugin::<ShadowFaceCamera>::default(),
                UniformComponentPlugin::<ShadowFaceUniform>::default(),
            ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<render::ShadowLayerViews>()
            .add_systems(RenderStartup, render::init_shadow_pipelines)
            .add_systems(
                Render,
                render::prepare_layer_views.in_set(RenderSystems::PrepareResources),
            )
            .add_systems(
                Core3d,
                render::shadow_face_pass
                    .after(Core3dSystems::Prepass)
                    .before(Core3dSystems::MainPass),
            );
    }

    fn finish(&self, app: &mut App) {
        if !shaders_available(app) || !app.world().contains_resource::<Assets<Image>>() {
            return;
        }
        load_shader_library!(app, "shaders/shadow_kernel.wesl");
        load_shader_library!(app, "shaders/vsm.wesl");
        cubemaps::create_placeholder(app);
        // Cameras must be in place before their frusta are computed this frame.
        app.add_systems(
            PostUpdate,
            cameras::place_shadow_cameras
                .after(TransformSystems::Propagate)
                .before(VisibilitySystems::UpdateFrusta),
        );
    }
}
