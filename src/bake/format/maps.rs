//! The sun maps and sky probes sections of a `.dynlight` payload: what moving meshes read for
//! the static scene's shadows of baked suns and skies.

use bevy::math::{UVec3, Vec3, Vec4};

use super::super::sky_probes::MAX_SKY_PROBES;
use super::codec::{Reader, Writer};
use super::types::{SkyProbes, SunMap};
use super::{FormatError, MAX_SUN_MAP_RESOLUTION};

/// Bytes per sun map before its words: resolution, rows and the word count.
const SUN_MAP_BYTES: usize = 60;

pub(super) fn encode(w: &mut Writer, sun_maps: &[SunMap], sky_probes: Option<&SkyProbes>) {
    w.count(sun_maps.len());
    for map in sun_maps {
        w.u32(map.resolution);
        for value in map.rows.iter().flat_map(|row| row.to_array()) {
            w.f32(value);
        }
        w.words(&map.texels);
    }

    w.u32(u32::from(sky_probes.is_some()));
    if let Some(probes) = sky_probes {
        for value in probes.origin.to_array() {
            w.f32(value);
        }
        w.f32(probes.spacing);
        for count in probes.counts.to_array() {
            w.u32(count);
        }
        w.words(&probes.words);
    }
}

/// Reads the sun maps (none, or one per sun) and the sky probes (only with a sky).
pub(super) fn decode(
    r: &mut Reader,
    suns: usize,
    skies: usize,
) -> Result<(Vec<SunMap>, Option<SkyProbes>), FormatError> {
    let map_count = r.count(SUN_MAP_BYTES)?;
    if map_count != 0 && map_count != suns {
        return Err(FormatError::Invalid("sun map count"));
    }
    let mut sun_maps = Vec::with_capacity(map_count);
    for _ in 0..map_count {
        let resolution = r.u32()?;
        let mut rows = [Vec4::ZERO; 3];
        for row in &mut rows {
            *row = Vec4::new(r.f32()?, r.f32()?, r.f32()?, r.f32()?);
        }
        let texels = r.words()?;
        let valid = (1..=MAX_SUN_MAP_RESOLUTION).contains(&resolution)
            && rows.iter().all(|row| row.is_finite())
            && texels.len() == SunMap::words(resolution);
        if !valid {
            return Err(FormatError::Invalid("sun map"));
        }
        sun_maps.push(SunMap {
            rows,
            resolution,
            texels,
        });
    }

    let sky_probes = match r.u32()? {
        0 => None,
        1 if skies > 0 => {
            let origin = Vec3::new(r.f32()?, r.f32()?, r.f32()?);
            let spacing = r.f32()?;
            let counts = UVec3::new(r.u32()?, r.u32()?, r.u32()?);
            let words = r.words()?;
            let probes = counts.cmpge(UVec3::ONE).all()
                && counts.as_dvec3().element_product() <= MAX_SKY_PROBES as f64;
            let valid = origin.is_finite()
                && spacing.is_finite()
                && spacing > 0.0
                && probes
                && words.len() == SkyProbes::probe_count(counts) * SkyProbes::PROBE_WORDS;
            if !valid {
                return Err(FormatError::Invalid("sky probes"));
            }
            Some(SkyProbes {
                origin,
                spacing,
                counts,
                words,
            })
        }
        _ => return Err(FormatError::Invalid("sky probes")),
    };
    Ok((sun_maps, sky_probes))
}
