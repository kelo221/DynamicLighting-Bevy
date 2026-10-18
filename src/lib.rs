//! DynLight: Henry de Jongh's Dynamic Lighting for Bevy.
//!
//! Shadows are baked per texel on the CPU, one bit per (triangle, light). Color, intensity,
//! effects and light shapes are evaluated live in our own shaders, so baked lights can still
//! flicker, pulse and change color. None of Bevy's lights or lighting shaders are used.
//!
//! Add [`DynamicLightingPlugin`], spawn [`light::DynamicLight`]s and draw meshes with
//! [`surface::DynLitMaterial`]. Mark static meshes [`bake::BakeStatic`], send a
//! [`bake::BakeRequest`] to bake them into a `.dynlight` file, and load that file into
//! [`baked_lighting::ActiveBake`] to apply it. The crate is organized as vertical slices, one
//! module per feature; see `AGENTS.md`.

pub mod bake;
pub mod baked_lighting;
pub mod cookies;
pub mod debug;
pub mod dynamic_geometry;
pub mod effects;
pub mod environment;
pub mod fog;
pub mod gltf_import;
pub mod light;
pub mod light_clusters;
pub mod realtime_shadows;
pub mod shared;
pub mod surface;

use bevy::prelude::*;

/// Common imports.
pub mod prelude {
    pub use crate::DynamicLightingPlugin;
    pub use crate::bake::{
        BakeFinished, BakePreset, BakeRequest, BakeSettings, BakeStatic, BakeStatus,
    };
    pub use crate::baked_lighting::ActiveBake;
    pub use crate::cookies::LightCookie;
    pub use crate::debug::LightGizmos;
    pub use crate::effects::{LightEffect, LightEffectKind, LightEffectState};
    pub use crate::environment::{NoSkyBackground, SkyEnvironment};
    pub use crate::fog::{FogPostProcess, VolumetricLight, VolumetricShape};
    pub use crate::light::{
        DynamicLight, Illumination, LightBounce, LightType, ShadowMode, Shimmer, Transparency,
        sun_light,
    };
    pub use crate::shared::{
        BakeId, DynamicLightingSettings, FeatureSelection, RuntimeQuality, ShaderFeatures,
        SubgroupUse, TriangleIdSource,
    };
    pub use crate::surface::{DynLitMaterial, ShadingModel};
}

/// Adds every DynLight slice.
pub struct DynamicLightingPlugin;

impl Plugin for DynamicLightingPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            shared::SharedPlugin,
            effects::EffectsPlugin,
            light::LightPlugin,
            light_clusters::LightClustersPlugin,
            cookies::CookiesPlugin,
            environment::EnvironmentPlugin,
            surface::SurfacePlugin,
            bake::BakePlugin,
            baked_lighting::BakedLightingPlugin,
            dynamic_geometry::DynamicGeometryPlugin,
            realtime_shadows::RealtimeShadowsPlugin,
            fog::FogPlugin,
            gltf_import::GltfImportPlugin,
            debug::DebugPlugin,
        ));
    }
}
