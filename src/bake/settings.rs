//! What gets baked, and how finely.

use bevy::prelude::*;

use crate::light::BounceCompression;

/// Marks a mesh as static scenery: it casts baked shadows and receives baked lighting. Static
/// meshes must not move, deform or change their index order after baking.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[reflect(Component, Default, Debug, PartialEq, Clone)]
pub struct BakeStatic;

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
    /// penumbras at a higher bake cost (texels fully lit or fully shadowed take 3 to 5 rays).
    /// Preview bakes use at most [`PREVIEW_AREA_LIGHT_SAMPLES`].
    pub area_light_samples: u32,
}

/// Area light samples per texel in Preview bakes.
pub const PREVIEW_AREA_LIGHT_SAMPLES: u32 = 32;

impl Default for BakeSettings {
    fn default() -> Self {
        Self {
            pixel_density: 128.0,
            max_lightmap_size: 2048,
            bounce_compression: BounceCompression::Bits8,
            area_light_samples: 64,
        }
    }
}
