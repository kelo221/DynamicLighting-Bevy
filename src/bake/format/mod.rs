//! The `.dynlight` file: a small uncompressed header, then a gzip-compressed payload.
//!
//! ```text
//! 0   8   magic "DYNLIGHT"
//! 8   4   format version (u32 LE), see FORMAT_VERSION
//! 12  8   payload length in bytes, uncompressed (u64 LE)
//! 20  4   CRC-32 (IEEE) of the uncompressed payload (u32 LE)
//! 24  ..  gzip stream of the payload
//! ```
//!
//! The payload is little-endian; lengths are u64 counts. Sections in order:
//! settings (flags u32 [bit 0 bounce], dynamic geometry u32 [0 LightingOnly, 1 DistanceCubes,
//! 2 Angular], pixel density f32, max lightmap size u32, distance cube resolution u32);
//! lights (count; id u64, origin 3×f32, radius f32, bounce bits per pixel u32 [0 = none], shape
//! u32 [0 point-like, 1 rectangle, 2 tube, 3 sun, 4 sky], rotation 4×f32 (x y z w), size 2×f32
//! [a sun's angular diameter in degrees, 0], shadow bits per texel u32 [1, 2, 4 or 8]; suns,
//! then skies, come last); light BVH (count; 8 words per node); meshes (count; id u64, content
//! hash u64, resolution u32, words); distance cube words; sun maps (count [0 or one per sun];
//! resolution u32, rows 12×f32, words); sky probes (u32 [0 none, 1 with a sky], then origin
//! 3×f32, spacing f32, counts 3×u32, words); stats (meshes u32, then triangles, texels, rays,
//! lights removed, bounce rays, triangle words as u64).

mod codec;
mod lights;
mod maps;
mod types;

use std::fmt;
use std::io::{Read, Write};

use flate2::{Compression, GzBuilder, read::GzDecoder};

use crate::light::LightShape;
use crate::shared::{BakeId, DynamicGeometryMode};

use super::light_bvh::LightBvhNode;
use codec::{Reader, Writer};
pub use types::{BakeStats, BakedLight, BakedMesh, BakedScene, BakedSettings, SkyProbes, SunMap};

pub const MAGIC: [u8; 8] = *b"DYNLIGHT";
/// Bumped on every layout change; loaders reject other versions. Version 2 added each light's
/// area shape and shadow bits per texel, version 3 suns and their sun maps, version 4 skies,
/// 8-bit shadows and the sky probes, version 5 whether each mesh casts shadows.
pub const FORMAT_VERSION: u32 = 5;
const HEADER_BYTES: usize = 24;
/// The largest sun map side a file may hold.
pub const MAX_SUN_MAP_RESOLUTION: u32 = 8192;

/// Why a `.dynlight` file couldn't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatError {
    NotDynlight,
    UnsupportedVersion { found: u32, supported: u32 },
    Truncated,
    Decompress(String),
    ChecksumMismatch,
    Invalid(&'static str),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDynlight => write!(f, "not a .dynlight file (bad magic)"),
            Self::UnsupportedVersion { found, supported } => write!(
                f,
                ".dynlight format version {found} is not supported (this build reads version \
                 {supported}); re-bake the scene"
            ),
            Self::Truncated => write!(f, ".dynlight file is truncated or has corrupt lengths"),
            Self::Decompress(error) => write!(f, ".dynlight payload doesn't decompress: {error}"),
            Self::ChecksumMismatch => write!(f, ".dynlight payload checksum mismatch"),
            Self::Invalid(what) => write!(f, ".dynlight file has an invalid {what}"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Serializes a baked scene. The same scene always gives the same bytes.
pub fn encode(scene: &BakedScene) -> Vec<u8> {
    let payload = encode_payload(scene);
    let mut out = Vec::with_capacity(HEADER_BYTES + payload.len() / 4);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&crc32fast::hash(&payload).to_le_bytes());
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(out, Compression::default());
    encoder
        .write_all(&payload)
        .and_then(|_| encoder.finish())
        .expect("writing to a Vec cannot fail")
}

/// Parses a `.dynlight` file, checking magic, version, length and checksum.
pub fn decode(bytes: &[u8]) -> Result<BakedScene, FormatError> {
    if bytes.len() < HEADER_BYTES || bytes[..8] != MAGIC {
        return Err(if bytes.len() >= 8 && bytes[..8] != MAGIC {
            FormatError::NotDynlight
        } else {
            FormatError::Truncated
        });
    }
    let mut header = Reader::new(&bytes[8..HEADER_BYTES]);
    let version = header.u32()?;
    if version != FORMAT_VERSION {
        return Err(FormatError::UnsupportedVersion {
            found: version,
            supported: FORMAT_VERSION,
        });
    }
    let length = header.u64()?;
    let checksum = header.u32()?;
    let mut payload = Vec::new();
    GzDecoder::new(&bytes[HEADER_BYTES..])
        .take(length.saturating_add(1))
        .read_to_end(&mut payload)
        .map_err(|error| FormatError::Decompress(error.to_string()))?;
    if payload.len() as u64 != length {
        return Err(FormatError::Truncated);
    }
    if crc32fast::hash(&payload) != checksum {
        return Err(FormatError::ChecksumMismatch);
    }
    decode_payload(&payload)
}

fn encode_payload(scene: &BakedScene) -> Vec<u8> {
    let mut w = Writer::default();
    let settings = &scene.settings;
    w.u32(u32::from(settings.bounce));
    w.u32(match settings.dynamic_geometry {
        DynamicGeometryMode::LightingOnly => 0,
        DynamicGeometryMode::DistanceCubes => 1,
        DynamicGeometryMode::Angular => 2,
    });
    w.f32(settings.pixel_density);
    w.u32(settings.max_lightmap_size);
    w.u32(settings.distance_cube_resolution);

    lights::encode(&mut w, &scene.lights);

    w.words(bytemuck::cast_slice(&scene.light_bvh));

    w.count(scene.meshes.len());
    for mesh in &scene.meshes {
        w.u64(mesh.id.0);
        w.u64(mesh.content_hash);
        w.u32(mesh.resolution);
        w.u32(u32::from(mesh.occludes));
        w.words(&mesh.triangles);
    }

    w.words(&scene.distance_cubes);

    maps::encode(&mut w, &scene.sun_maps, scene.sky_probes.as_ref());

    let stats = &scene.stats;
    w.u32(stats.meshes);
    for value in [
        stats.triangles,
        stats.texels,
        stats.rays,
        stats.lights_removed,
        stats.bounce_rays,
        stats.triangle_words,
    ] {
        w.u64(value);
    }
    w.bytes
}

fn decode_payload(payload: &[u8]) -> Result<BakedScene, FormatError> {
    let mut r = Reader::new(payload);
    let flags = r.u32()?;
    if flags > 1 {
        return Err(FormatError::Invalid("settings flag"));
    }
    let dynamic_geometry = match r.u32()? {
        0 => DynamicGeometryMode::LightingOnly,
        1 => DynamicGeometryMode::DistanceCubes,
        2 => DynamicGeometryMode::Angular,
        _ => return Err(FormatError::Invalid("dynamic geometry mode")),
    };
    let settings = BakedSettings {
        bounce: flags & 1 != 0,
        dynamic_geometry,
        pixel_density: r.f32()?,
        max_lightmap_size: r.u32()?,
        distance_cube_resolution: r.u32()?,
    };

    let lights = lights::decode(&mut r)?;

    let bvh_words = r.words()?;
    if !bvh_words.len().is_multiple_of(8) {
        return Err(FormatError::Invalid("light BVH"));
    }
    let light_bvh: Vec<LightBvhNode> = bvh_words
        .as_chunks::<8>()
        .0
        .iter()
        .map(|node| bytemuck::pod_read_unaligned(bytemuck::cast_slice(node)))
        .collect();

    let mesh_count = r.count(32)?;
    let mut meshes = Vec::with_capacity(mesh_count);
    for _ in 0..mesh_count {
        let (id, content_hash, resolution) = (BakeId(r.u64()?), r.u64()?, r.u32()?);
        let occludes = match r.u32()? {
            0 => false,
            1 => true,
            _ => return Err(FormatError::Invalid("mesh shadow flag")),
        };
        meshes.push(BakedMesh {
            id,
            content_hash,
            resolution,
            triangles: r.words()?,
            occludes,
        });
    }

    let distance_cubes = r.words()?;

    let suns = lights.iter().filter(|light| light.shape.sun().is_some());
    let skies = lights.iter().filter(|light| light.shape == LightShape::Sky);
    let (sun_maps, sky_probes) = maps::decode(&mut r, suns.count(), skies.count())?;

    let stats = BakeStats {
        meshes: r.u32()?,
        triangles: r.u64()?,
        texels: r.u64()?,
        rays: r.u64()?,
        lights_removed: r.u64()?,
        bounce_rays: r.u64()?,
        triangle_words: r.u64()?,
    };
    if r.remaining() != 0 {
        return Err(FormatError::Invalid("trailing payload"));
    }
    Ok(BakedScene {
        settings,
        lights,
        light_bvh,
        meshes,
        distance_cubes,
        sun_maps,
        sky_probes,
        stats,
    })
}

#[cfg(test)]
mod tests;
