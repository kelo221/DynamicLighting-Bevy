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
    /// Distance in meters from the light's position where the light ends completely. Area
    /// lights are measured from their position too: past their shape they reach `radius` minus
    /// half the shape's diagonal ([`Self::shape_reach`]). Suns ignore it: they reach everything.
    pub radius: f32,
    /// Extra decay within the radius for an artistic look, such as a bright desk lamp. 0 is
    /// physically based; 1 roughly matches the falloff to the radius. Suns ignore it.
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
    /// Rectangle: width (local X) and height (local Y) of the panel in meters. Tube: length
    /// (local X); `y` is unused. Also the start of frustum fog. Zero makes an area light a point.
    /// Baked area lights get soft shadows traced over this shape; moving, turning or resizing
    /// one drops it to realtime lighting until the next bake.
    pub size: Vec2,
    /// Sun: how wide the sun's disk looks, in degrees (the real sun is 0.53°), 0 to
    /// [`Self::MAX_ANGULAR_DIAMETER`]. Larger disks cast softer baked shadows; 0 gives Henry's
    /// hard one-bit shadows. Changing it drops a baked sun to realtime lighting until the next
    /// bake.
    pub angular_diameter: f32,
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
            size: Vec2::ZERO,
            angular_diameter: Self::SUN_ANGULAR_DIAMETER,
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
    /// The apparent size of the sun's disk in degrees.
    pub const SUN_ANGULAR_DIAMETER: f32 = 0.53;
    /// The largest sun disk, in degrees. Wider disks would light around corners that a disk at
    /// infinity can't.
    pub const MAX_ANGULAR_DIAMETER: f32 = 10.0;

    /// Spot and discoball lights need `outer_cutoff >= inner_cutoff` and a non-zero
    /// `outer_cutoff`; area lights a size that isn't negative and a radius beyond their shape;
    /// suns an angular diameter from 0 to [`Self::MAX_ANGULAR_DIAMETER`]. Invalid lights are
    /// disabled in every render path.
    pub fn is_valid(&self) -> bool {
        match self.light_type {
            LightType::Spot | LightType::Discoball => {
                self.outer_cutoff >= self.inner_cutoff && self.outer_cutoff != 0.0
            }
            LightType::Rectangle | LightType::Tube => {
                self.size.cmpge(Vec2::ZERO).all() && self.shape_reach() > 0.0
            }
            LightType::Sun => (0.0..=Self::MAX_ANGULAR_DIAMETER).contains(&self.angular_diameter),
            _ => true,
        }
    }

    /// Half the sun's angular diameter in radians; 0 for every other type.
    pub fn angular_radius(&self) -> f32 {
        match self.light_type {
            LightType::Sun => (self.angular_diameter * 0.5).to_radians(),
            _ => 0.0,
        }
    }

    /// Half extents of an area light's shape in its local X and Y: the panel, or the tube with
    /// zero height. Zero for every other type.
    pub fn half_extents(&self) -> Vec2 {
        match self.light_type {
            LightType::Rectangle => self.size.max(Vec2::ZERO) / 2.0,
            LightType::Tube => Vec2::new(self.size.x.max(0.0) / 2.0, 0.0),
            _ => Vec2::ZERO,
        }
    }

    /// How far the shape reaches from the light's position: half its diagonal, zero for
    /// point-like types.
    pub fn extent(&self) -> f32 {
        self.half_extents().length()
    }

    /// How far the light reaches past its shape: the radius minus [`Self::extent`], so that an
    /// area light, like every other light, ends within `radius` of its position. The radius
    /// itself for point-like types.
    pub fn shape_reach(&self) -> f32 {
        self.radius - self.extent()
    }
}
