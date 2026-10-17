//! Light authoring and the per-frame light buffer: the [`DynamicLight`] component, its types
//! and options, packing into [`GpuLight`](crate::shared::GpuLight) records, the realtime budget,
//! moved baked lights, presets, the WESL light-type and area-light functions, the LTC table for
//! rectangle-light specular ([`AREA_LIGHT_LUT`]), and a CPU reference of that shader math.
//!
//! Shaders import the light functions as `dynlight::light::shaders::{light_types, area, ltc}`.

pub mod area;
#[cfg(test)]
mod area_record_tests;
#[cfg(test)]
mod area_tests;
mod budget;
mod buffer;
mod component;
mod features;
mod kinds;
pub mod ltc;
mod ltc_image;
#[cfg(test)]
mod ltc_tests;
pub mod mirror;
mod pack;
mod placement;
mod presets;
#[cfg(test)]
pub(crate) mod shader_tests;
mod spatial;
mod systems;
#[cfg(test)]
mod tests;

use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;
use bevy::shader::load_shader_library;
use bevy::transform::TransformSystems;

pub use area::AreaLightShape;
pub use budget::{RealtimeCandidate, select_realtime};
pub use buffer::{
    BakedLightSlots, BakedSlot, LIGHT_BUFFER, LightAttachments, PackedLights, RealtimeViewer,
};
pub use component::DynamicLight;
pub use features::light_features;
pub use kinds::{
    BounceCompression, Illumination, LightBounce, LightType, ShadowMode, Shimmer, Transparency,
};
pub use ltc_image::{AREA_LIGHT_LUT, lut_image};
pub use pack::{LightPose, PackContext, bounce_color, pack};
pub use placement::BakedInPlace;
pub use presets::{SUN_DISTANCE, SUN_RADIUS, sun_light};
pub use spatial::{cookie_projection_scale, type_parameters};
pub use systems::quality_level;

use crate::effects::EffectsSystems;
use crate::shared::shaders_available;

/// Ordering of the light buffer work in `PostUpdate`.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LightSystems {
    /// [`BakedInPlace`] is brought up to date, before camera frusta are computed.
    Locate,
    /// Other slices assign cookie and shadow-cubemap slots in [`LightAttachments`].
    Attach,
    /// Lights are packed into [`PackedLights`] and uploaded.
    Pack,
}

pub struct LightPlugin;

impl Plugin for LightPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<DynamicLight>()
            .init_resource::<BakedLightSlots>()
            .init_resource::<LightAttachments>()
            .init_resource::<PackedLights>()
            .init_resource::<placement::LightPlacement>()
            .configure_sets(
                PostUpdate,
                (LightSystems::Attach, LightSystems::Pack)
                    .chain()
                    .after(EffectsSystems)
                    .after(TransformSystems::Propagate)
                    .after(VisibilitySystems::UpdateFrusta)
                    .after(VisibilitySystems::VisibilityPropagate),
            )
            .configure_sets(
                PostUpdate,
                LightSystems::Locate
                    .after(TransformSystems::Propagate)
                    .before(VisibilitySystems::UpdateFrusta),
            )
            .add_systems(
                PostUpdate,
                placement::mark_baked_in_place.in_set(LightSystems::Locate),
            )
            .add_systems(
                PostUpdate,
                (
                    systems::pack_lights,
                    (buffer::upload_lights, features::note_light_features),
                )
                    .chain()
                    .in_set(LightSystems::Pack),
            );
    }

    fn finish(&self, app: &mut App) {
        ltc_image::insert_lut(app);
        if shaders_available(app) {
            load_shader_library!(app, "shaders/light_types.wesl");
            load_shader_library!(app, "shaders/ltc.wesl");
            load_shader_library!(app, "shaders/area.wesl");
            buffer::create_light_buffer(app);
        }
    }
}
