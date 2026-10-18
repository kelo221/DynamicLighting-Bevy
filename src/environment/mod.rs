//! The sky: a [`LightType::Sky`] light with a [`SkyEnvironment`] image lights every surface
//! from all directions and is drawn behind the scene.
//!
//! When the image loads, a background task projects it to spherical harmonics for diffuse
//! light (packed into the light buffer as `shared::GpuSkyIrradiance`), prefilters it into a GGX
//! cube for Metallic reflections ([`SKY_SPECULAR`]) and builds a mip-mapped copy for the
//! background ([`SKY_BACKGROUND`]). Like Henry's lights, the sky's color, intensity and
//! intensity effects stay live, and turning its entity turns it.
//!
//! Shader modules: `dynlight::environment::shaders::sky`; the background pass is
//! `background.wesl`.

pub mod equirect;
mod frame;
pub mod images;
pub mod mirror;
pub mod prefilter;
mod render;
pub mod sh;
#[cfg(test)]
pub(crate) mod shader_tests;
mod systems;
#[cfg(test)]
mod tests;

use bevy::asset::{AssetPlugin, embedded_asset, uuid_handle};
use bevy::core_pipeline::Core3d;
use bevy::core_pipeline::core_3d::{main_opaque_pass_3d, main_transparent_pass_3d};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_resource::SpecializedRenderPipelines;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::load_shader_library;

pub use frame::{SkyFrame, first_sky};

use crate::light::{DynamicLight, LightSystems};
use crate::shared::shaders_available;

/// The prefiltered specular cube every lit material binds: mip level `m` is the sky seen by
/// perceptual roughness `m / (levels − 1)`, in the sky's own frame. White while no sky has an
/// image, black while an image sky is being built.
pub const SKY_SPECULAR: Handle<Image> = uuid_handle!("2f6c1d8e-93a4-4b7f-b0e5-6a1c9d3e7f28");
/// The sky image with mips that the background pass draws.
pub const SKY_BACKGROUND: Handle<Image> = uuid_handle!("7a3e5b90-1c4d-4e8f-a6b2-d95f0c3e1a47");

/// The image of a [`LightType::Sky`] light: an equirectangular image of linear radiance,
/// mapped like Blender's world texture (its center column looks along +X). EXR and Radiance HDR
/// files load with Bevy's `exr` and `hdr` features. The image must keep its CPU data, which is
/// the default. Building the sky's lighting takes a moment after the image loads; until then
/// the sky adds no light.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Debug, Clone, PartialEq)]
#[require(DynamicLight = DynamicLight::sky())]
pub struct SkyEnvironment {
    pub image: Handle<Image>,
    /// Radiance above this is scaled down before the sky lights the scene; the background
    /// still shows it. Set it when a `LightType::Sun` light stands in for a sun in the image,
    /// so that sun isn't counted twice and its tiny bright disk doesn't speckle reflections.
    pub max_radiance: Option<f32>,
}

impl SkyEnvironment {
    pub fn new(image: Handle<Image>) -> Self {
        Self {
            image,
            max_radiance: None,
        }
    }
}

/// Put on a camera that should keep its clear color instead of drawing the sky behind the
/// scene.
#[derive(Component, ExtractComponent, Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Component, Debug, Clone, Default, PartialEq)]
#[extract_app(RenderApp)]
pub struct NoSkyBackground;

/// Whether the sky's lighting is still on its way: its image is loading or being built.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SkyStatus {
    pub building: bool,
}

/// The sky the background pass draws, if any; extracted to the render world.
#[derive(Resource, ExtractResource, Clone, Debug, Default, PartialEq)]
#[extract_app(RenderApp)]
pub struct SkyBackground(pub Option<SkyFrame>);

pub struct EnvironmentPlugin;

impl Plugin for EnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SkyEnvironment>()
            .register_type::<NoSkyBackground>()
            .init_resource::<SkyBackground>()
            .init_resource::<SkyStatus>()
            .add_systems(
                PostUpdate,
                systems::note_sky_background.after(LightSystems::Pack),
            );
        // The background pass needs the asset server and a renderer.
        if !app.is_plugin_added::<AssetPlugin>() {
            return;
        }
        embedded_asset!(app, "shaders/background.wesl");
        app.add_plugins((
            ExtractComponentPlugin::<NoSkyBackground>::default(),
            ExtractResourcePlugin::<SkyBackground>::default(),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SpecializedRenderPipelines<render::BackgroundPipeline>>()
            .add_systems(RenderStartup, render::init_background_pipeline)
            .add_systems(
                Render,
                (
                    render::prepare_background_pipelines.in_set(RenderSystems::Prepare),
                    render::prepare_background_uniform.in_set(RenderSystems::PrepareResources),
                ),
            )
            .add_systems(
                Core3d,
                render::background_pass
                    .after(main_opaque_pass_3d)
                    .before(main_transparent_pass_3d),
            );
    }

    fn finish(&self, app: &mut App) {
        if !app.world().contains_resource::<Assets<Image>>() {
            return;
        }
        systems::insert_placeholders(app);
        app.init_resource::<systems::SkyBuilds>()
            .add_systems(PostUpdate, systems::build_skies.before(LightSystems::Pack));
        if shaders_available(app) {
            load_shader_library!(app, "shaders/sky.wesl");
        }
    }
}
