//! Realtime variance shadow cubemaps for lights with `ShadowMode::Realtime`: six depth-only
//! face views per light, drawn by our own render phase and `Core3d` pass, turned into blurred
//! distance moments and sampled by the lit material.
//!
//! Cubes are cached: a light keeps its slot in the cube array, and a face renders again only
//! when its light moves or a shadow caster it can see changes. Lights that use their baked
//! shadows draw only meshes without baked data. Material changes are followed for material
//! types with a [`CasterMaterialPlugin`]; `DynLitMaterial` has one.
//!
//! Shader modules: `dynlight::realtime_shadows::shaders::{vsm, shadow_kernel}`; the face pass
//! entries are `shadow_moments.wesl` and `shadow_blur.wesl`.

pub mod casters;
mod cubemaps;
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
#[cfg(test)]
mod tests;
mod tracking;
mod views;

use bevy::asset::{AssetPlugin, embedded_asset};
use bevy::camera::visibility::VisibilitySystems;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::pbr::{DrawDepthOnlyPrepass, DrawPrepass, MeshPipeline};
use bevy::prelude::*;
use bevy::render::extract_component::UniformComponentPlugin;
use bevy::render::render_phase::{AddRenderCommand, BinnedRenderPhasePlugin, DrawFunctions};
use bevy::render::{
    ExtractSchedule, Render, RenderApp, RenderDebugFlags, RenderStartup, RenderSystems,
};
use bevy::shader::load_shader_library;
use bevy::transform::TransformSystems;

pub use cubemaps::{
    SHADOW_CUBEMAP_FORMATS, SHADOW_CUBEMAPS, ShadowCubemapCapacity, cubemap_array,
    shadow_cubemap_format,
};
pub use materials::{CasterMaterial, CasterMaterialPlugin};
pub use views::{ShadowCubemapStats, ShadowFace, ShadowFaceUniform};

use crate::light::LightSystems;
use crate::shared::shaders_available;

pub struct RealtimeShadowsPlugin;

impl Plugin for RealtimeShadowsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShadowCubemapStats>();
        // Face views and the cube array need a renderer; headless apps skip realtime shadows.
        if !app.is_plugin_added::<AssetPlugin>() {
            return;
        }
        embedded_asset!(app, "shaders/shadow_moments.wesl");
        embedded_asset!(app, "shaders/shadow_blur.wesl");
        let feedback = tracking::ShadowFeedback::default();
        app.init_resource::<views::ShadowFaces>()
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
            .init_resource::<render::ShadowLayerViews>()
            .init_resource::<render::ShadowFaceBindGroups>()
            .init_resource::<DrawFunctions<phase::ShadowFace3d>>()
            .add_render_command::<phase::ShadowFace3d, phase::Reported<DrawPrepass>>()
            .add_render_command::<phase::ShadowFace3d, phase::Reported<DrawDepthOnlyPrepass>>()
            .insert_resource(feedback)
            .add_systems(RenderStartup, render::init_shadow_pipelines)
            .add_systems(ExtractSchedule, extract::extract_shadow_faces)
            .add_systems(
                Render,
                (
                    render::prepare_layer_views.in_set(RenderSystems::PrepareResources),
                    render::prepare_face_bind_groups.in_set(RenderSystems::PrepareBindGroups),
                    queue::queue_shadow_faces.in_set(RenderSystems::QueueMeshes),
                ),
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
        app.init_resource::<tracking::TouchedCasters>();
        // Caster bounds and visibility must be final before the changes are collected; the
        // casters faces draw are marked visible along with what cameras see.
        app.add_systems(
            PostUpdate,
            (
                (tracking::track_casters, views::place_shadow_faces)
                    .chain()
                    .after(TransformSystems::Propagate)
                    .after(VisibilitySystems::CalculateBounds)
                    .after(VisibilitySystems::VisibilityPropagate)
                    .after(LightSystems::Locate)
                    .before(VisibilitySystems::CheckVisibility),
                views::cull_casters
                    .in_set(VisibilitySystems::CheckVisibility)
                    .after(views::place_shadow_faces),
            ),
        );
    }
}
