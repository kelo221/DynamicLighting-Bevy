//! The two baked-data buffers the shaders read.
//!
//! ```text
//! triangles   every baked mesh's `dynamic_triangles` words, back to back
//! aux         header (12 words), receiver records (4 words each), light BVH nodes (8 words
//!             each), distance cubes (DISTANCE_CUBE_WORDS per BVH light), sun map records
//!             (16 words each), sun map texels, the sky probe record (8 words) and probes
//! ```
//!
//! A sun map record holds the map's three world-to-map rows (12 f32), its resolution and the aux
//! word its texels start at. Sun map `k` belongs to the `k`-th baked sun, light slot
//! `BAKED_LIGHTS − distant count + k`: the suns come first of the distant lights. The sky probe
//! record holds the grid's origin (3 f32), spacing (f32), counts (3 u32) and the aux word its
//! probes start at; header word `SKY_PROBES` is 0 without one.
//!
//! A mesh finds its record through `MeshTag`: tag `n > 0` is receiver `n − 1`, tag 0 means "not
//! baked". Henry's triangle offsets stay relative to the mesh's first word, so "0 = no data"
//! still holds; the shader adds the receiver's `triangles_base`.
//! `baked_lighting/shaders/baked_data.wesl` mirrors every constant here.

use bevy::platform::collections::HashMap;

use crate::bake::{SkyProbes, SunMap};
use crate::shared::BakeId;

/// Aux header word indices.
pub mod header {
    /// First word of the receiver records.
    pub const RECEIVERS: usize = 0;
    pub const RECEIVER_COUNT: usize = 1;
    /// First word of the light BVH nodes.
    pub const BVH: usize = 2;
    pub const BVH_NODE_COUNT: usize = 3;
    /// First word of the distance cubes.
    pub const CUBES: usize = 4;
    /// Baked lights with a distance cube (all in the BVH, or none).
    pub const CUBE_COUNT: usize = 5;
    /// [`super::flags`].
    pub const FLAGS: usize = 6;
    /// Number of baked light slots.
    pub const BAKED_LIGHTS: usize = 7;
    /// First word of the sun map records.
    pub const SUN_MAPS: usize = 8;
    /// Baked suns with a sun map (all of them, or none).
    pub const SUN_MAP_COUNT: usize = 9;
    /// The sky probe record, or 0.
    pub const SKY_PROBES: usize = 10;
}

/// Bits of the aux header's flags word.
pub mod flags {
    /// Triangle light entries are 3 words (light, shadow, bounce) instead of 2.
    pub const BOUNCE_LAYOUT: u32 = 1;
    /// The distance cubes carry Henry's +0.25 m `DistanceCubes` bias.
    pub const CUBE_BIAS: u32 = 2;
    /// The distance cubes are meant for angular filtering.
    pub const CUBE_ANGULAR: u32 = 4;
}

pub const AUX_HEADER_WORDS: usize = 12;
pub const RECEIVER_WORDS: usize = 4;
pub const BVH_NODE_WORDS: usize = 8;
pub const SUN_MAP_RECORD_WORDS: usize = 16;
/// Word offsets inside a sun map record, after its rows.
pub const SUN_MAP_RESOLUTION: usize = 12;
pub const SUN_MAP_TEXELS: usize = 13;
pub const SKY_PROBE_RECORD_WORDS: usize = 8;
/// Word offsets inside the sky probe record, after its origin.
pub const SKY_PROBE_SPACING: usize = 3;
pub const SKY_PROBE_COUNTS: usize = 4;
pub const SKY_PROBE_WORDS: usize = 7;

/// One baked mesh as the buffers see it.
#[derive(Clone, Copy, Debug)]
pub struct ReceiverInput<'a> {
    pub id: BakeId,
    pub content_hash: u64,
    /// Shadow map side in texels.
    pub resolution: u32,
    pub triangles: &'a [u32],
}

/// Where a baked mesh's data went, for tagging its entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receiver {
    /// The `MeshTag` value that selects this record.
    pub tag: u32,
    pub content_hash: u64,
    pub triangle_count: u32,
}

/// Contents of both buffers plus the receiver lookup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedBuffers {
    pub triangles: Vec<u32>,
    pub aux: Vec<u32>,
    pub receivers: HashMap<BakeId, Receiver>,
}

/// Scene-wide inputs besides the meshes.
#[derive(Clone, Copy, Debug, Default)]
pub struct SceneInput<'a> {
    pub baked_lights: u32,
    /// Light BVH nodes, [`BVH_NODE_WORDS`] words each.
    pub bvh: &'a [u32],
    /// Distance cubes, `cube_words` per light in BVH order.
    pub cubes: &'a [u32],
    pub cube_count: u32,
    /// One per baked sun, in slot order, or none.
    pub sun_maps: &'a [SunMap],
    pub sky_probes: Option<&'a SkyProbes>,
    pub flags: u32,
}

impl BakedBuffers {
    /// Buffers with no bake: every mesh is unbaked and there are no baked lights.
    pub fn empty() -> Self {
        Self::build(core::iter::empty(), SceneInput::default())
    }

    pub fn build<'a>(
        meshes: impl IntoIterator<Item = ReceiverInput<'a>>,
        scene: SceneInput<'_>,
    ) -> Self {
        debug_assert_eq!(scene.bvh.len() % BVH_NODE_WORDS, 0);
        let mut triangles = Vec::new();
        let mut records = Vec::new();
        let mut receivers = HashMap::default();
        for mesh in meshes {
            let triangle_count = mesh.triangles.first().map_or(0, |&offset| offset / 4);
            let tag = records.len() as u32 / RECEIVER_WORDS as u32 + 1;
            records.extend([triangles.len() as u32, mesh.resolution, triangle_count, 0]);
            triangles.extend_from_slice(mesh.triangles);
            receivers.insert(
                mesh.id,
                Receiver {
                    tag,
                    content_hash: mesh.content_hash,
                    triangle_count,
                },
            );
        }
        // Storage buffers can't be empty.
        if triangles.is_empty() {
            triangles.push(0);
        }

        let mut aux = vec![0; AUX_HEADER_WORDS];
        aux[header::RECEIVERS] = aux.len() as u32;
        aux[header::RECEIVER_COUNT] = (records.len() / RECEIVER_WORDS) as u32;
        aux.extend_from_slice(&records);
        aux[header::BVH] = aux.len() as u32;
        aux[header::BVH_NODE_COUNT] = (scene.bvh.len() / BVH_NODE_WORDS) as u32;
        aux.extend_from_slice(scene.bvh);
        aux[header::CUBES] = aux.len() as u32;
        aux[header::CUBE_COUNT] = scene.cube_count;
        aux.extend_from_slice(scene.cubes);
        aux[header::FLAGS] = scene.flags;
        aux[header::BAKED_LIGHTS] = scene.baked_lights;

        aux[header::SUN_MAPS] = aux.len() as u32;
        aux[header::SUN_MAP_COUNT] = scene.sun_maps.len() as u32;
        let mut texels = aux.len() + scene.sun_maps.len() * SUN_MAP_RECORD_WORDS;
        for map in scene.sun_maps {
            let rows = map.rows.iter().flat_map(|row| row.to_array());
            aux.extend(rows.map(f32::to_bits));
            aux.extend([map.resolution, texels as u32, 0, 0]);
            texels += map.texels.len();
        }
        for map in scene.sun_maps {
            aux.extend_from_slice(&map.texels);
        }

        if let Some(probes) = scene.sky_probes {
            aux[header::SKY_PROBES] = aux.len() as u32;
            aux.extend(probes.origin.to_array().map(f32::to_bits));
            aux.push(probes.spacing.to_bits());
            aux.extend(probes.counts.to_array());
            aux.push((aux.len() + 1) as u32);
            aux.extend_from_slice(&probes.words);
        }

        Self {
            triangles,
            aux,
            receivers,
        }
    }

    /// The receiver record for `tag`, as `[triangles_base, resolution, triangle_count, flags]`.
    pub fn record(&self, tag: u32) -> Option<&[u32]> {
        let index = tag.checked_sub(1)? as usize;
        if index >= self.aux[header::RECEIVER_COUNT] as usize {
            return None;
        }
        let start = self.aux[header::RECEIVERS] as usize + index * RECEIVER_WORDS;
        Some(&self.aux[start..start + RECEIVER_WORDS])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffers_are_valid_storage() {
        let buffers = BakedBuffers::empty();
        assert_eq!(buffers.triangles, [0]);
        assert_eq!(buffers.aux.len(), AUX_HEADER_WORDS);
        assert_eq!(buffers.aux[header::RECEIVER_COUNT], 0);
        assert_eq!(buffers.aux[header::SKY_PROBES], 0);
        assert_eq!(buffers.record(1), None);
        assert_eq!(buffers.record(0), None);
    }

    #[test]
    fn meshes_are_concatenated_with_their_base() {
        let a = [8, 0, 0, 5, 4, 0, 0, 7, 0, 0];
        let b = [4, 1, 1, 6, 1, 3, 0];
        let bvh = [0u32; 8];
        let maps = [1, 2].map(|resolution| SunMap {
            rows: [bevy::math::Vec4::splat(resolution as f32); 3],
            resolution,
            texels: vec![resolution * 10; SunMap::words(resolution)],
        });
        let probes = SkyProbes {
            origin: bevy::math::Vec3::new(1.0, 2.0, 3.0),
            spacing: 0.5,
            counts: bevy::math::UVec3::new(1, 2, 1),
            words: vec![11, 12, 13, 14],
        };
        let buffers = BakedBuffers::build(
            [
                ReceiverInput {
                    id: BakeId(10),
                    content_hash: 1,
                    resolution: 64,
                    triangles: &a,
                },
                ReceiverInput {
                    id: BakeId(20),
                    content_hash: 2,
                    resolution: 32,
                    triangles: &b,
                },
            ],
            SceneInput {
                baked_lights: 4,
                bvh: &bvh,
                cubes: &[7, 8],
                cube_count: 1,
                sun_maps: &maps,
                sky_probes: Some(&probes),
                flags: flags::BOUNCE_LAYOUT,
            },
        );
        assert_eq!(buffers.triangles.len(), a.len() + b.len());
        assert_eq!(&buffers.triangles[a.len()..], b);

        let first = buffers.receivers[&BakeId(10)];
        let second = buffers.receivers[&BakeId(20)];
        assert_eq!((first.tag, first.triangle_count), (1, 2));
        assert_eq!((second.tag, second.content_hash), (2, 2));
        assert_eq!(buffers.record(1), Some(&[0, 64, 2, 0][..]));
        assert_eq!(buffers.record(2), Some(&[a.len() as u32, 32, 1, 0][..]));
        assert_eq!(buffers.record(3), None);

        let aux = &buffers.aux;
        assert_eq!(
            aux[header::BVH],
            (AUX_HEADER_WORDS + 2 * RECEIVER_WORDS) as u32
        );
        assert_eq!(aux[header::BVH_NODE_COUNT], 1);
        assert_eq!(aux[header::CUBES], aux[header::BVH] + 8);
        let cubes = aux[header::CUBES] as usize;
        assert_eq!(aux[cubes..cubes + 2], [7, 8]);
        assert_eq!(aux[header::FLAGS], flags::BOUNCE_LAYOUT);
        assert_eq!(aux[header::BAKED_LIGHTS], 4);

        assert_eq!(aux[header::SUN_MAPS], aux[header::CUBES] + 2);
        assert_eq!(aux[header::SUN_MAP_COUNT], 2);
        for (k, map) in maps.iter().enumerate() {
            let record = aux[header::SUN_MAPS] as usize + k * SUN_MAP_RECORD_WORDS;
            let row = f32::from_bits(aux[record + 11]);
            assert_eq!(row, map.resolution as f32);
            assert_eq!(aux[record + SUN_MAP_RESOLUTION], map.resolution);
            let texels = aux[record + SUN_MAP_TEXELS] as usize;
            assert_eq!(aux[texels..texels + map.texels.len()], map.texels);
        }
        let last = aux[aux[header::SUN_MAPS] as usize + SUN_MAP_RECORD_WORDS + SUN_MAP_TEXELS];
        let record = aux[header::SKY_PROBES] as usize;
        assert_eq!(
            last as usize + 2,
            record,
            "the 2² map's 2 words end the maps"
        );
        assert_eq!(f32::from_bits(aux[record + 2]), 3.0);
        assert_eq!(f32::from_bits(aux[record + SKY_PROBE_SPACING]), 0.5);
        assert_eq!(aux[record + SKY_PROBE_COUNTS..][..3], [1, 2, 1]);
        let first = aux[record + SKY_PROBE_WORDS] as usize;
        assert_eq!(first, record + SKY_PROBE_RECORD_WORDS);
        assert_eq!(aux[first..], [11, 12, 13, 14]);
    }
}
