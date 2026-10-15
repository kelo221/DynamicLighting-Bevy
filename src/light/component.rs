//! The `DynamicLight` authoring component.

use bevy::prelude::*;

use super::kinds::{Illumination, LightBounce, LightType, ShadowMode, Shimmer, Transparency};

/// A light rendered by DynLight's own shaders. It is not a Bevy light and Bevy's lighting never
/// sees it.
///
/// Baked lights get per-texel shadows from the bake; lights marked [`Self::realtime`] (and baked
/// lights that moved) are evaluated every frame without baked shadows. Animate the intensity
/// with `LightEffect`; add volumetric fog with `VolumetricLight`.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, Debug, PartialEq, Clone)]
#[require(Transform, Visibility)]
pub struct DynamicLight {
    pub color: Color,
    /// Brightness. Doesn't change the range.
    pub intensity: f32,
    /// Distance in meters where the light ends completely.
    pub radius: f32,
    /// Extra decay within the radius for an artistic look, such as a bright desk lamp. 0 is
    /// physically based; 1 roughly matches the falloff to the radius.
    pub falloff: f32,
    /// Never baked: evaluated every frame without baked shadows, so it can move freely.
    pub realtime: bool,
    pub light_type: LightType,
    /// Spot and discoball: angle in degrees from the axis where the light starts to fade.
    pub inner_cutoff: f32,
    /// Spot and discoball: angle in degrees where the light ends. Must be at least
    /// `inner_cutoff` and above zero, or the light turns off. Also the angle of cone fog.
    pub outer_cutoff: f32,
    /// Wave, interference, rotor, shock and disco: how fast the pattern moves. Negative
    /// reverses it.
    pub wave_speed: f32,
    /// Wave, interference, rotor, shock and disco: how many waves or blades; higher is denser.
    pub wave_frequency: f32,
    /// Wave, interference, rotor, shock and disco: phase offset, so lights don't move in sync.
    pub wave_offset: f32,
    /// Rotor: size of the blob of light at the center, -1 to 1. Negative makes it a shadow.
    pub rotor_center: f32,
    /// Disco: how fast the spots move vertically. Negative reverses it.
    pub disco_vertical_speed: f32,
    pub shadows: ShadowMode,
    pub illumination: Illumination,
    pub transparency: Transparency,
    pub bounce: LightBounce,
    pub shimmer: Shimmer,
    /// Shimmer: scale of the projected blocks; larger values give smaller blocks.
    pub shimmer_scale: f32,
    /// Shimmer: how dim a block can become, where 0 is fully off and 1 does nothing.
    pub shimmer_modifier: f32,
}

impl Default for DynamicLight {
    fn default() -> Self {
        Self {
            color: Color::WHITE,
            intensity: 2.0,
            radius: 4.0,
            falloff: 0.0,
            realtime: false,
            light_type: LightType::Point,
            inner_cutoff: 26.0,
            outer_cutoff: 30.0,
            wave_speed: 1.0,
            wave_frequency: 1.0,
            wave_offset: 0.0,
            rotor_center: 0.1,
            disco_vertical_speed: 1.0,
            shadows: ShadowMode::Raytraced,
            illumination: Illumination::Direct,
            transparency: Transparency::Disabled,
            bounce: LightBounce::default(),
            shimmer: Shimmer::None,
            shimmer_scale: 12.25,
            shimmer_modifier: 0.8,
        }
    }
}

impl DynamicLight {
    /// Spot and discoball lights need `outer_cutoff >= inner_cutoff` and a non-zero
    /// `outer_cutoff`; invalid ones are disabled in every render path.
    pub fn is_valid(&self) -> bool {
        !matches!(self.light_type, LightType::Spot | LightType::Discoball)
            || (self.outer_cutoff >= self.inner_cutoff && self.outer_cutoff != 0.0)
    }
}
