//! The lights section of a `.dynlight` payload: one [`LIGHT_BYTES`] record per baked light, in
//! light buffer order (the BVH's lights, then the suns, then the skies).

use bevy::math::{Quat, Vec2, Vec3};

use crate::light::{AreaLightShape, BounceCompression, LightShape, LightType, SunDisk};
use crate::shared::BakeId;

use super::FormatError;
use super::codec::{Reader, Writer};
use super::types::BakedLight;

/// Bytes per light record.
const LIGHT_BYTES: usize = 60;

/// Where a light's kind may appear: every light of a group comes before the next group's.
fn group(shape: &LightShape) -> u8 {
    match shape {
        LightShape::Point | LightShape::Area(_) => 0,
        LightShape::Sun(_) => 1,
        LightShape::Sky => 2,
    }
}

pub(super) fn encode(w: &mut Writer, lights: &[BakedLight]) {
    w.count(lights.len());
    for light in lights {
        w.u64(light.id.0);
        for value in light.origin.to_array() {
            w.f32(value);
        }
        w.f32(light.radius);
        w.u32(light.bounce_compression.map_or(0, |c| c.bits_per_pixel()));
        let (kind, rotation, size) = match light.shape {
            LightShape::Point => (0, Quat::IDENTITY, Vec2::ZERO),
            LightShape::Area(area) => (
                if area.kind == LightType::Tube { 2 } else { 1 },
                area.rotation,
                area.size,
            ),
            LightShape::Sun(sun) => (3, sun.rotation, Vec2::new(sun.angular_diameter, 0.0)),
            LightShape::Sky => (4, Quat::IDENTITY, Vec2::ZERO),
        };
        w.u32(kind);
        for value in rotation.to_array().into_iter().chain(size.to_array()) {
            w.f32(value);
        }
        w.u32(light.shadow_bits_per_texel);
    }
}

pub(super) fn decode(r: &mut Reader) -> Result<Vec<BakedLight>, FormatError> {
    let light_count = r.count(LIGHT_BYTES)?;
    let mut lights: Vec<BakedLight> = Vec::with_capacity(light_count);
    for _ in 0..light_count {
        let id = BakeId(r.u64()?);
        let origin = Vec3::new(r.f32()?, r.f32()?, r.f32()?);
        let radius = r.f32()?;
        let bounce_compression = match r.u32()? {
            0 => None,
            8 => Some(BounceCompression::Bits8),
            6 => Some(BounceCompression::Bits6),
            5 => Some(BounceCompression::Bits5),
            4 => Some(BounceCompression::Bits4),
            _ => return Err(FormatError::Invalid("bounce compression")),
        };
        let kind = r.u32()?;
        let rotation = Quat::from_xyzw(r.f32()?, r.f32()?, r.f32()?, r.f32()?);
        let size = Vec2::new(r.f32()?, r.f32()?);
        if !(rotation.is_finite() && size.is_finite()) {
            return Err(FormatError::Invalid("light shape"));
        }
        let area = |kind| {
            LightShape::Area(AreaLightShape {
                kind,
                rotation,
                size,
            })
        };
        let shape = match kind {
            0 => LightShape::Point,
            1 => area(LightType::Rectangle),
            2 => area(LightType::Tube),
            3 => LightShape::Sun(SunDisk {
                rotation,
                angular_diameter: size.x,
            }),
            4 => LightShape::Sky,
            _ => return Err(FormatError::Invalid("light shape")),
        };
        if lights
            .last()
            .is_some_and(|last| group(&last.shape) > group(&shape))
        {
            return Err(FormatError::Invalid("light order"));
        }
        let shadow_bits_per_texel = r.u32()?;
        if ![1, 2, 4, 8].contains(&shadow_bits_per_texel) {
            return Err(FormatError::Invalid("shadow bits per texel"));
        }
        lights.push(BakedLight {
            id,
            origin,
            radius,
            shape,
            bounce_compression,
            shadow_bits_per_texel,
        });
    }
    Ok(lights)
}
