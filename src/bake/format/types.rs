//! The baked scene as the runtime consumes it.

use bevy::math::{Vec3, Vec4};

use crate::light::{BounceCompression, LightShape};
use crate::shared::{BakeId, DynamicGeometryMode};

use super::super::light_bvh::LightBvhNode;

/// Everything a bake produced. Lights, BVH and distance cubes are in BVH order; meshes are
/// sorted by [`BakeId`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedScene {
    pub settings: BakedSettings,
    /// Baked lights in BVH order, then the suns (which aren't in the BVH) by [`BakeId`]:
    /// `lights[i]` belongs in GPU light buffer slot `i`, and the `lightIndex` of every triangle
    /// light entry indexes this list.
    pub lights: Vec<BakedLight>,
    /// The light BVH over all lights but the suns; empty when there are none.
    pub light_bvh: Vec<LightBvhNode>,
    pub meshes: Vec<BakedMesh>,
    /// Per BVH light [`super::super::mirror::DISTANCE_CUBE_WORDS`] words: 6 faces × 64²
    /// closest-hit distances in meters, two f16 per word. Empty when
    /// [`BakedSettings::dynamic_geometry`] is `LightingOnly`.
    pub distance_cubes: Vec<u32>,
    /// One per sun, in light order; empty when [`BakedSettings::dynamic_geometry`] is
    /// `LightingOnly`.
    pub sun_maps: Vec<SunMap>,
    pub stats: BakeStats,
}

impl BakedScene {
    /// Lights in the light BVH; the suns follow them.
    pub fn local_light_count(&self) -> usize {
        self.lights.len() - self.sun_count()
    }

    pub fn sun_count(&self) -> usize {
        self.lights
            .iter()
            .rev()
            .take_while(|light| light.shape.is_distant())
            .count()
    }
}

/// A sun's view of the static scene: the depth of the first surface along its rays over a
/// square grid, so meshes without baked shadows can tell where static geometry shades them.
#[derive(Clone, Debug, PartialEq)]
pub struct SunMap {
    /// World to map space: coordinate `i` of point `p` is `rows[i].xyz · p + rows[i].w`. x and y
    /// run from 0 to 1 across the map, z from 0 at the plane facing the sun to 1 at the far side
    /// of the static scene, along the rays.
    pub rows: [Vec4; 3],
    /// Texels per side.
    pub resolution: u32,
    /// `resolution²` depths in map space as unorm16, rows of increasing y, two per word: texel
    /// `2i` in the low half of word `i`. A ray that hits nothing stores 1.
    pub texels: Vec<u32>,
}

impl SunMap {
    /// Words holding `resolution²` texels.
    pub fn words(resolution: u32) -> usize {
        (resolution as usize * resolution as usize).div_ceil(2)
    }
}

/// Scene-wide facts the runtime needs to pick shader variants.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedSettings {
    /// Any light baked bounce lighting: every triangle light entry is 3 words
    /// (`DL_BOUNCE_LAYOUT`) instead of 2.
    pub bounce: bool,
    /// The dynamic geometry mode the distance cubes were baked for, which selects the shader
    /// filter. Stored distances are exact in every mode.
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
/// moved (realtime, unshadowed) while its shape differs from `shape` or, except for suns, its
/// position differs from `origin` or its radius is larger than `radius`.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedLight {
    pub id: BakeId,
    pub origin: Vec3,
    pub radius: f32,
    /// The shape the light's shadows were traced for.
    pub shape: LightShape,
    /// The storage precision of this light's bounce texels (channel bits 17–19), or `None`
    /// when the light has no baked bounce.
    pub bounce_compression: Option<BounceCompression>,
    /// Bits per shadow texel (channel bits 20–22): 1 for hard shadows, 4 for soft ones (area
    /// lights and suns with a disk).
    pub shadow_bits_per_texel: u32,
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
