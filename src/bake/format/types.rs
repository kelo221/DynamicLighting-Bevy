//! The baked scene as the runtime consumes it.

use bevy::math::Vec3;

use crate::light::BounceCompression;
use crate::shared::{BakeId, DynamicGeometryMode};

use super::super::light_bvh::LightBvhNode;

/// Everything a bake produced. Lights, BVH and distance cubes are in BVH order; meshes are
/// sorted by [`BakeId`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedScene {
    pub settings: BakedSettings,
    /// Baked lights in BVH order: `lights[i]` belongs in GPU light buffer slot `i`, and the
    /// `lightIndex` of every triangle light entry indexes this list.
    pub lights: Vec<BakedLight>,
    /// The light BVH; empty when there are no baked lights.
    pub light_bvh: Vec<LightBvhNode>,
    pub meshes: Vec<BakedMesh>,
    /// Per light (BVH order) [`super::super::mirror::DISTANCE_CUBE_WORDS`] words: 6 faces × 64²
    /// closest-hit distances in meters, two f16 per word. Empty when
    /// [`BakedSettings::dynamic_geometry`] is `LightingOnly`.
    pub distance_cubes: Vec<u32>,
    pub stats: BakeStats,
}

/// Scene-wide facts the runtime needs to pick shader variants.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedSettings {
    /// Any light baked bounce lighting: every triangle light entry is 3 words
    /// (`DL_BOUNCE_LAYOUT`) instead of 2.
    pub bounce: bool,
    /// The dynamic geometry mode the distance cubes were baked for. `DistanceCubes` adds Henry's
    /// +0.25 m bias to every stored distance; `Angular` stores exact distances.
    pub dynamic_geometry: DynamicGeometryMode,
    /// Texels per meter used for the shadow maps.
    pub pixel_density: f32,
    pub max_lightmap_size: u32,
    /// Texels per side of each distance cube face (64).
    pub distance_cube_resolution: u32,
}

impl Default for BakedSettings {
    fn default() -> Self {
        Self {
            bounce: false,
            dynamic_geometry: DynamicGeometryMode::default(),
            pixel_density: 128.0,
            max_lightmap_size: 2048,
            distance_cube_resolution: 64,
        }
    }
}

/// A baked light. The runtime matches it to a `DynamicLight` by `id`, and treats the light as
/// moved (realtime, unshadowed) while its position differs from `origin`.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedLight {
    pub id: BakeId,
    pub origin: Vec3,
    pub radius: f32,
    /// The storage precision of this light's bounce texels (channel bits 17–19), or `None`
    /// when the light has no baked bounce.
    pub bounce_compression: Option<BounceCompression>,
}

/// One static mesh's baked data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakedMesh {
    pub id: BakeId,
    /// [`crate::shared::mesh_content_hash`] of the mesh the bake saw.
    pub content_hash: u64,
    /// Shadow map side in texels: the shader's `lightmap_resolution` (UV1 · resolution gives
    /// texel coordinates).
    pub resolution: u32,
    /// The mesh's `dynamic_triangles` words, read as [`crate::bake::mirror::TriangleView`] does.
    pub triangles: Vec<u32>,
}

impl BakedMesh {
    /// Number of triangles (headers) in [`Self::triangles`].
    pub fn triangle_count(&self) -> u32 {
        self.triangles.first().map_or(0, |&offset| offset / 4)
    }
}

/// Deterministic bake statistics (no timings, so files stay byte-identical).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BakeStats {
    pub meshes: u32,
    pub triangles: u64,
    /// Texel centers inside triangles, each counted once.
    pub texels: u64,
    /// Shadow rays traced.
    pub rays: u64,
    /// Light entries dropped because the light never reached the triangle.
    pub lights_removed: u64,
    /// Bounce rays traced.
    pub bounce_rays: u64,
    /// Total `dynamic_triangles` words over all meshes.
    pub triangle_words: u64,
}
