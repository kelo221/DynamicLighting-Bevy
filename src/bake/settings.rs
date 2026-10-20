//! What gets baked, and how finely.

use bevy::prelude::*;

use crate::light::BounceCompression;

/// Marks a mesh as static scenery: it casts baked shadows and receives baked lighting. Static
/// meshes must not move, deform or change their index order after baking.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[reflect(Component, Default, Debug, PartialEq, Clone)]
pub struct BakeStatic;

/// A [`BakeStatic`] mesh whose material is blended (alpha modes Blend, Premultiplied, Add and
/// Multiply), such as glass. The bake traces through it, as realtime shadows skip it. The
/// surface slice keeps it in step with the mesh's `DynLitMaterial`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlendedSurface;

/// A [`BakeStatic`] mesh whose material or base color texture is still loading, so whether it is
/// blended, its alpha mask and the color it reflects aren't known yet. Bakes wait for these
/// meshes. The surface slice keeps it in step with the mesh's `DynLitMaterial`.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfacePending;

/// Bake resolution and storage settings.
#[derive(Resource, Reflect, Clone, Debug, PartialEq)]
#[reflect(Resource, Default, Debug, Clone, PartialEq)]
pub struct BakeSettings {
    /// Shadow texels per meter. A mesh's shadow map side is `ceil(sqrt(density² × area))`.
    pub pixel_density: f32,
    /// Largest shadow map side per mesh, in texels.
    pub max_lightmap_size: u32,
    /// Bounce storage precision for lights that don't choose their own.
    pub bounce_compression: BounceCompression,
    /// Shadow rays per penumbra texel for rectangle and tube lights; more give smoother
    /// penumbras at a higher bake cost. Preview bakes use at most [`PREVIEW_AREA_LIGHT_SAMPLES`].
    pub area_light_samples: u32,
    /// Of those, the rays every texel in reach of an area light takes, after the 3 to 5 probe
    /// rays to its corners, ends and center, before it may count as fully lit or fully shadowed.
    /// They catch thin occluders, such as poles and railings, that pass between the probes. 0
    /// trusts the probes alone, as Preview bakes do.
    pub area_light_min_samples: u32,
    /// Side in texels of each baked sun's sun map, the depth map of the static scene seen from
    /// the sun that gives moving meshes its static shadows. Its texels span the whole scene, so
    /// large scenes need more. Preview bakes use at most [`PREVIEW_SUN_MAP_RESOLUTION`].
    pub sun_map_resolution: u32,
    /// Rays per texel, and per probe direction, to a baked sky: how finely sky shadows grade.
    /// Preview bakes use at most [`PREVIEW_SKY_SAMPLES`].
    pub sky_samples: u32,
    /// Meters between the sky probes that give moving meshes the baked sky's shadows. Large
    /// scenes space them further apart to stay within
    /// [`MAX_SKY_PROBES`](super::MAX_SKY_PROBES).
    pub sky_probe_spacing: f32,
    /// Bounce light takes on the color of the surfaces it leaves ([`super::BakeAlbedo`]),
    /// stored as three channels per bounce texel. Off, bounce light is the light's bounce color
    /// alone, as in Henry's package, and its photons reflect as if white.
    pub colored_bounce: bool,
}

/// Area light samples per texel in Preview bakes, which also skip the minimum samples.
pub const PREVIEW_AREA_LIGHT_SAMPLES: u32 = 32;
/// Sun map side in Preview bakes.
pub const PREVIEW_SUN_MAP_RESOLUTION: u32 = 512;
/// Sky samples in Preview bakes.
pub const PREVIEW_SKY_SAMPLES: u32 = 16;

impl Default for BakeSettings {
    fn default() -> Self {
        Self {
            pixel_density: 128.0,
            max_lightmap_size: 2048,
            bounce_compression: BounceCompression::Bits8,
            area_light_samples: 64,
            area_light_min_samples: 16,
            sun_map_resolution: super::sun_map::SUN_MAP_RESOLUTION,
            sky_samples: 64,
            sky_probe_spacing: 1.0,
            colored_bounce: false,
        }
    }
}
