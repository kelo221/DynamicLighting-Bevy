//! The enums that configure a [`super::DynamicLight`].

use bevy::prelude::*;

/// The shape of a light. Discriminants are the values stored in the GPU channel bits.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
#[repr(u32)]
pub enum LightType {
    /// Shines equally in every direction.
    #[default]
    Point = 0,
    /// A cone along the light's forward axis, between `inner_cutoff` and `outer_cutoff`.
    Spot = 1,
    /// Many small spot cones in a mirror-ball pattern.
    Discoball = 2,
    /// Concentric rings travelling outwards.
    Wave = 3,
    /// Bands of light and shadow around the light's up axis.
    Interference = 4,
    /// Rotating blades of light around the light's up axis, like a warning beacon.
    Rotor = 5,
    /// Rings of uneven brightness, like an electric shock.
    Shock = 6,
    /// Spots of light scrolling around and up and down.
    Disco = 7,
    /// An area light: a `size.x` by `size.y` panel in the light's local XY plane, lighting only
    /// the side its forward axis points to, like a ceiling panel. Like every type, it is
    /// shadowed from its position and ends within `radius` of it.
    Rectangle = 8,
    /// An area light: a `size.x` long segment along the light's local X axis, lighting all
    /// around, like a fluorescent tube. Like every type, it is shadowed from its position and
    /// ends within `radius` of it.
    Tube = 9,
    /// A distant light: parallel light from a disk at infinity, shining along the light's
    /// forward axis (`Transform::forward`). It has no position or radius and reaches everything;
    /// `angular_diameter` is the disk's size, which softens its baked shadows.
    Sun = 10,
}

impl LightType {
    pub const ALL: [Self; 11] = [
        Self::Point,
        Self::Spot,
        Self::Discoball,
        Self::Wave,
        Self::Interference,
        Self::Rotor,
        Self::Shock,
        Self::Disco,
        Self::Rectangle,
        Self::Tube,
        Self::Sun,
    ];

    /// The type with GPU discriminant `bits`, if there is one.
    pub fn from_bits(bits: u32) -> Option<Self> {
        Self::ALL.get(bits as usize).copied()
    }

    /// Light comes from a shape instead of a point: [`Self::Rectangle`] and [`Self::Tube`].
    pub fn is_area(self) -> bool {
        matches!(self, Self::Rectangle | Self::Tube)
    }

    /// Light from infinitely far away, without a position or radius: [`Self::Sun`]. Distant
    /// lights reach every fragment, so they are kept out of the light BVH and the realtime
    /// light grid, and each fragment visits them in a list of their own.
    pub fn is_distant(self) -> bool {
        matches!(self, Self::Sun)
    }
}

/// How a light casts shadows.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum ShadowMode {
    /// Shadows of static geometry are baked. The light must not move: while it is away from
    /// where it was baked it becomes a realtime light without shadows.
    #[default]
    Raytraced,
    /// Baked shadows plus a realtime shadow cubemap, so moving meshes cast shadows too.
    /// Expensive; limited by `DynamicLightingSettings::shadow_cubemap_budget`.
    Realtime,
}

/// Whether the bake adds indirect light.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum Illumination {
    /// Direct light only: shadows per texel, no bounce.
    #[default]
    Direct,
    /// Direct light plus one baked bounce off lit surfaces, stored per texel.
    SingleBounce,
}

/// Whether baked shadows let light through alpha-tested textures (alpha above 0.5).
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum Transparency {
    /// Every surface blocks light. Fastest and most precise.
    #[default]
    Disabled,
    /// Light passes through baked surfaces where their base color alpha is below the cutoff
    /// (0.5, or the material's mask cutoff).
    Enabled,
    /// Same as `Enabled` in this port: shadow rays alpha-test directly, so Henry's alternative
    /// photon-cube sampling has no counterpart.
    EnabledMax,
}

/// A pattern of flickering blocks projected onto lit surfaces.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
#[repr(u32)]
pub enum Shimmer {
    #[default]
    None = 0,
    /// Blocks that smoothly brighten and darken, like caustics from water.
    Water = 1,
    /// Blocks that change randomly, like wavering firelight.
    Random = 2,
}

/// Bits per pixel used to store baked bounce lighting. Fewer bits save memory at the cost of
/// banding, which is softened by dithering.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum BounceCompression {
    /// Lossless.
    #[default]
    Bits8,
    /// 20% less memory.
    Bits6,
    /// 34% less memory.
    Bits5,
    /// 50% less memory.
    Bits4,
}

impl BounceCompression {
    pub const fn bits_per_pixel(self) -> u32 {
        match self {
            Self::Bits8 => 8,
            Self::Bits6 => 6,
            Self::Bits5 => 5,
            Self::Bits4 => 4,
        }
    }
}

/// Baked bounce settings, used when the light's illumination is
/// [`Illumination::SingleBounce`].
#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(Default, Debug, PartialEq, Clone)]
pub struct LightBounce {
    /// Tint of the bounce light. Alpha blends from the light's own color (0) to this color (1).
    pub color: Color,
    /// Visibility of the bounce light at runtime, 0 to 1. Doesn't require a re-bake.
    pub modifier: f32,
    /// Strength of the bounce light. Requires a re-bake.
    pub intensity: f32,
    /// Samples per texel during the bake; more is smoother and slower. At least 2.
    pub samples: u32,
    /// Storage precision; `None` uses the bake's default.
    pub compression: Option<BounceCompression>,
}

impl Default for LightBounce {
    fn default() -> Self {
        Self {
            color: Color::srgba(1.0, 1.0, 1.0, 0.0),
            modifier: 1.0,
            intensity: 1.0,
            samples: 32,
            compression: None,
        }
    }
}
