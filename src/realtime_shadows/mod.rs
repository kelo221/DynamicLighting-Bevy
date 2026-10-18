//! Realtime variance shadow cubemaps for lights with `ShadowMode::Realtime`: six depth-only
//! face views per light, drawn by our own render phase and pass (all views at once, before the
//! cameras), turned into blurred distance moments and sampled by the lit material.
//!
//! Cubes are cached: a light keeps its slot in the cube array, and a face renders again only
//! when its light moves or a shadow caster it can see changes. Lights that use their baked
//! shadows draw only meshes without baked data. Material changes are followed for material
//! types with a [`CasterMaterialPlugin`]; `DynLitMaterial` has one.
//!
//! One sun with `ShadowMode::Realtime` gets cascades instead: orthographic views along its rays
//! over the camera's view, drawn by the same phase and pass into their own array and cached the
//! same way (see [`cascades`]).
//!
//! Shader modules: `dynlight::realtime_shadows::shaders::{vsm, sun_vsm, shadow_kernel}`; the
//! face pass entries are `shadow_moments.wesl` and `shadow_blur.wesl`.

#[cfg(test)]
mod cascade_tests;
pub mod cascades;
pub mod casters;
mod choice;
mod cubemaps;
mod driver;
mod extract;
#[cfg(test)]
mod face_tests;
pub mod faces;
mod materials;
pub mod mirror;
mod phase;
mod queue;
mod render;
#[cfg(test)]
pub(crate) mod shader_tests;
pub mod slots;
mod sun;
pub mod sun_mirror;
mod targets;
#[cfg(test)]
mod tests;
mod tracking;
mod views;

use bevy::asset::{AssetPlugin, embedded_asset};
use bevy::camera::visibility::VisibilitySystems;
use bevy::pbr::{DrawDepthOnlyPrepass, DrawPrepass, MeshPipeline};
use bevy::prelude::*;
use bevy::render::extract_component::UniformComponentPlugin;
use bevy::render::render_phase::{AddRenderCommand, BinnedRenderPhasePlugin, DrawFunctions};
use bevy::render::{
    ExtractSchedule, Render, RenderApp, RenderDebugFlags, RenderStartup, RenderSystems,
};
use bevy::shader::load_shader_library;
use bevy::transform::TransformSystems;

pub use choice::ShadowChoice;
pub use cubemaps::{
    SHADOW_CUBEMAP_FORMATS, SHADOW_CUBEMAPS, SUN_CASCADES, ShadowCubemapCapacity, cascade_array,
    cubemap_array, shadow_cubemap_format,
};
pub use materials::{CasterMaterial, CasterMaterialPlugin};
pub use sun::SunCascadeView;
pub use views::{ShadowCubemapStats, ShadowFace, ShadowFaceUniform};

use crate::light::LightSystems;
use crate::shared::shaders_available;

pub struct RealtimeShadowsPlugin;

impl Plugin for RealtimeShadowsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShadowCubemapStats>()
            .init_resource::<ShadowChoice>()
            .add_systems(
                PostUpdate,
                choice::choose_shadow_lights
                    .in_set(LightSystems::Attach)
                    .after(LightSystems::Locate),
            );
        // Face views and the cube array need a renderer; headless apps only choose the lights.
        if !app.is_plugin_added::<AssetPlugin>() {
            return;
        }
        embedded_asset!(app, "shaders/shadow_moments.wesl");
        embedded_asset!(app, "shaders/shadow_blur.wesl");
        let feedback = tracking::ShadowFeedback::default();
        app.init_resource::<views::ShadowFaces>()
            .init_resource::<sun::SunCascades>()
            .init_resource::<ShadowCubemapCapacity>()
            .init_resource::<tracking::CasterBounds>()
            .init_resource::<tracking::CasterChanges>()
            .insert_resource(feedback.clone())
            .add_plugins((
                UniformComponentPlugin::<ShadowFaceUniform>::default(),
                BinnedRenderPhasePlugin::<phase::ShadowFace3d, MeshPipeline>::new(
                    RenderDebugFlags::default(),
                ),
            ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<targets::ShadowLayerViews>()
            .init_resource::<targets::ShadowFaceBindGroups>()
            .init_resource::<DrawFunctions<phase::ShadowFace3d>>()
            .add_render_command::<phase::ShadowFace3d, phase::Reported<DrawPrepass>>()
            .add_render_command::<phase::ShadowFace3d, phase::Reported<DrawDepthOnlyPrepass>>()
            .insert_resource(feedback)
            .init_resource::<driver::PreprocessedShadowViews>()
            .add_schedule(driver::shadow_views_schedule())
            .add_systems(
                RenderStartup,
                (render::init_shadow_pipelines, driver::spawn_shadow_driver),
            )
            .add_systems(ExtractSchedule, extract::extract_shadow_faces)
            .add_systems(
                Render,
                (
                    targets::prepare_layer_views.in_set(RenderSystems::PrepareResources),
                    targets::prepare_face_bind_groups.in_set(RenderSystems::PrepareBindGroups),
                    queue::queue_shadow_faces.in_set(RenderSystems::QueueMeshes),
                ),
            )
            .add_systems(
                driver::ShadowViews,
                (driver::preprocess_shadow_views, render::shadow_face_pass).chain(),
            );
    }

    fn finish(&self, app: &mut App) {
        if !shaders_available(app) || !app.world().contains_resource::<Assets<Image>>() {
            return;
        }
        load_shader_library!(app, "shaders/shadow_kernel.wesl");
        load_shader_library!(app, "shaders/vsm.wesl");
        load_shader_library!(app, "shaders/sun_vsm.wesl");
        cubemaps::create_placeholder(app);
        app.init_resource::<tracking::TouchedCasters>();
        // Caster bounds and visibility must be final before the changes are collected; the
        // casters faces and cascades draw are marked visible along with what cameras see. The
        // cascades read the settle frames before the faces count them down.
        app.add_systems(
            PostUpdate,
            (
                (
                    tracking::track_casters,
                    sun::place_sun_cascades,
                    views::place_shadow_faces,
                )
                    .chain()
                    .in_set(LightSystems::Attach)
                    .after(TransformSystems::Propagate)
                    .after(VisibilitySystems::CalculateBounds)
                    .after(VisibilitySystems::VisibilityPropagate)
                    .after(LightSystems::Locate)
                    .after(choice::choose_shadow_lights)
                    .before(VisibilitySystems::CheckVisibility),
                (views::cull_casters, sun::cull_sun_casters)
                    .in_set(VisibilitySystems::CheckVisibility)
                    .after(views::place_shadow_faces),
            ),
        );
    }
}
