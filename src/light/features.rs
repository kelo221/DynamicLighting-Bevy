//! Which lighting paths of the lit shader the packed lights need ([`ShaderFeatures`]).

use bevy::prelude::*;

use super::buffer::PackedLights;
use super::kinds::LightType;
use crate::shared::{ChannelFlags, GpuLight, ShaderFeatures, ShaderFeaturesInUse};

/// The light features `lights` use, `realtime_count` of them realtime. `unbaked_meshes` is left
/// off; lights don't decide it.
pub fn light_features(lights: &[GpuLight], realtime_count: u32) -> ShaderFeatures {
    let mut features = ShaderFeatures {
        realtime_lights: realtime_count > 0,
        ..ShaderFeatures::NONE
    };
    for light in lights {
        let flags = ChannelFlags::unpack(light.channel);
        let area = flags.light_type == LightType::Rectangle as u32
            || flags.light_type == LightType::Tube as u32;
        features.area_lights |= area;
        features.light_patterns |=
            flags.cookie || (!area && flags.light_type != LightType::Point as u32);
        features.realtime_shadows |= flags.shadow_cubemap;
    }
    features
}

pub(super) fn note_light_features(
    packed: Res<PackedLights>,
    mut in_use: ResMut<ShaderFeaturesInUse>,
) {
    let used = light_features(&packed.lights, packed.globals.realtime_count);
    let in_use = &mut in_use.0;
    in_use.light_patterns |= used.light_patterns;
    in_use.area_lights |= used.area_lights;
    in_use.realtime_lights |= used.realtime_lights;
    in_use.realtime_shadows |= used.realtime_shadows;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(light_type: LightType, extra: u32) -> GpuLight {
        GpuLight {
            channel: ((light_type as u32) << ChannelFlags::TYPE_SHIFT) | extra,
            ..GpuLight::DISABLED
        }
    }

    #[test]
    fn point_lights_need_nothing() {
        let lights = [light(LightType::Point, 0), light(LightType::Point, 0)];
        assert_eq!(light_features(&lights, 0), ShaderFeatures::NONE);
    }

    #[test]
    fn each_light_kind_needs_its_feature() {
        let spot = light_features(&[light(LightType::Spot, 0)], 0);
        assert!(spot.light_patterns && !spot.area_lights);
        let tube = light_features(&[light(LightType::Tube, 0)], 0);
        assert!(tube.area_lights && !tube.light_patterns);
        let cookie = light_features(&[light(LightType::Point, ChannelFlags::COOKIE)], 0);
        assert!(cookie.light_patterns);
        let shadowed = light_features(&[light(LightType::Point, ChannelFlags::SHADOW_CUBEMAP)], 1);
        assert!(shadowed.realtime_shadows && shadowed.realtime_lights);
    }
}
