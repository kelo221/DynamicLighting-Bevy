//! Packs a [`DynamicLight`] into the [`GpuLight`] record the shaders read.

use bevy::color::{Color, ColorToComponents, Srgba};
use bevy::math::{Quat, Vec3};

use super::component::DynamicLight;
use super::kinds::{LightType, Shimmer};
use super::spatial::{cookie_projection_scale, type_parameters};
use crate::shared::{ChannelFlags, GpuLight};

/// Where a light is and which way it faces, from its global transform.
#[derive(Clone, Copy, Debug, Default)]
pub struct LightPose {
    pub position: Vec3,
    pub rotation: Quat,
}

/// Per-frame inputs that don't live on the light component.
#[derive(Clone, Copy, Debug)]
pub struct PackContext {
    /// Shader clock in seconds, for the animated light types.
    pub seconds: f32,
    /// The light's current effect multiplier, 1 without an effect.
    pub effect_intensity: f32,
    /// Evaluated without baked shadows.
    pub realtime: bool,
    /// Bounce bits per pixel of the light's baked bounce data, 1 to 8.
    pub bounce_bits_per_pixel: u32,
    /// Bits per texel of the light's baked shadows: 1, or more for an area light's soft shadows.
    pub shadow_bits_per_texel: u32,
    /// Slot in the cookie texture array, if the light has a cookie there.
    pub cookie_index: Option<u32>,
    /// Slot in the realtime shadow cubemap array, if the light renders one.
    pub shadow_cubemap_index: Option<u32>,
}

impl Default for PackContext {
    fn default() -> Self {
        Self {
            seconds: 0.0,
            effect_intensity: 1.0,
            realtime: true,
            bounce_bits_per_pixel: 8,
            shadow_bits_per_texel: 1,
            cookie_index: None,
            shadow_cubemap_index: None,
        }
    }
}

/// The shader record for `light`. Invalid lights, such as broken spot cones, give
/// [`GpuLight::DISABLED`].
pub fn pack(light: &DynamicLight, pose: LightPose, context: PackContext) -> GpuLight {
    if !light.is_valid() {
        return GpuLight::DISABLED;
    }
    let cookie = matches!(light.light_type, LightType::Spot) && context.cookie_index.is_some();
    let channel = ChannelFlags {
        realtime: context.realtime,
        light_type: light.light_type as u32,
        shimmer: light.shimmer as u32,
        shadow_cubemap: context.shadow_cubemap_index.is_some(),
        cookie,
        bounce_bits_per_pixel: context.bounce_bits_per_pixel,
        shadow_bits_per_texel: context.shadow_bits_per_texel,
    }
    .pack();
    let [gp1, gp2, mut gp3] = type_parameters(light, context.seconds);
    if cookie {
        gp3 = cookie_projection_scale(light.outer_cutoff);
    }
    let (shimmer_scale, shimmer_modifier) = match light.shimmer {
        Shimmer::None => (0.0, 0.0),
        Shimmer::Water => (light.shimmer_scale, 1.0 - light.shimmer_modifier),
        Shimmer::Random => (light.shimmer_scale, light.shimmer_modifier),
    };
    let forward = match light.light_type {
        // Henry stores the spot axis negated so `dot(to_light, forward)` is the cone cosine. A
        // sun's is the direction towards it, the same for every surface.
        LightType::Spot | LightType::Sun => pose.rotation * Vec3::Z,
        _ => pose.rotation * Vec3::NEG_Z,
    };
    // Area lights attenuate from the nearest point of their shape over the reach past it. Distant
    // lights have no radius: the shaders measure no distance to them, so any positive radius
    // leaves their attenuation at their intensity.
    let (radius_sqr, falloff) = if light.light_type.is_distant() {
        (f32::MAX, 0.0)
    } else {
        let reach = light.shape_reach();
        (reach * reach, reach * light.falloff * light.falloff)
    };
    GpuLight {
        position: pose.position.to_array(),
        radius_sqr,
        channel,
        intensity: light.intensity * context.effect_intensity,
        gp1,
        gp2,
        color: linear_rgb(light.color),
        gp3,
        up: (pose.rotation * Vec3::Y).to_array(),
        shimmer_scale,
        forward: forward.to_array(),
        shimmer_modifier,
        volumetric_intensity: 0.0,
        volumetric_visibility: 0.0,
        cookie_index: context.cookie_index.unwrap_or(0),
        shadow_cubemap_index: context.shadow_cubemap_index.unwrap_or(0),
        bounce_color: bounce_color(light),
        falloff,
    }
}

/// The bounce tint: the light color blended towards the bounce color by its alpha, in sRGB as
/// Henry does, scaled by the bounce modifier, then converted to linear.
pub fn bounce_color(light: &DynamicLight) -> [f32; 3] {
    let base = light.color.to_srgba();
    let tint = light.bounce.color.to_srgba();
    let mix = |from: f32, to: f32| (from + (to - from) * tint.alpha) * light.bounce.modifier;
    let mixed = Srgba::new(
        mix(base.red, tint.red),
        mix(base.green, tint.green),
        mix(base.blue, tint.blue),
        1.0,
    );
    linear_rgb(Color::Srgba(mixed))
}

fn linear_rgb(color: Color) -> [f32; 3] {
    color.to_linear().to_f32_array_no_alpha()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light::mirror;

    fn pose_looking_down() -> LightPose {
        LightPose {
            position: Vec3::new(0.0, 3.0, 0.0),
            rotation: Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2),
        }
    }

    #[test]
    fn spot_light_looking_down_lights_the_floor_below() {
        let light = DynamicLight {
            light_type: LightType::Spot,
            radius: 5.0,
            ..Default::default()
        };
        let record = pack(&light, pose_looking_down(), PackContext::default());
        assert!(mirror::radiance(&record, Vec3::new(0.0, 0.5, 0.0)) > 0.0);
        assert_eq!(mirror::radiance(&record, Vec3::new(0.0, 5.0, 0.0)), 0.0);
    }

    #[test]
    fn invalid_cones_are_disabled() {
        let light = DynamicLight {
            light_type: LightType::Spot,
            inner_cutoff: 40.0,
            outer_cutoff: 30.0,
            ..Default::default()
        };
        let record = pack(&light, LightPose::default(), PackContext::default());
        assert_eq!(record, GpuLight::DISABLED);
    }

    #[test]
    fn effect_scales_intensity_and_falloff_follows_henry() {
        let light = DynamicLight {
            intensity: 3.0,
            radius: 4.0,
            falloff: 0.5,
            ..Default::default()
        };
        let context = PackContext {
            effect_intensity: 0.5,
            ..Default::default()
        };
        let record = pack(&light, LightPose::default(), context);
        assert_eq!(record.intensity, 1.5);
        assert_eq!(record.radius_sqr, 16.0);
        assert_eq!(record.falloff, 1.0);
    }

    #[test]
    fn channel_carries_type_shimmer_and_realtime() {
        let light = DynamicLight {
            light_type: LightType::Rotor,
            shimmer: Shimmer::Water,
            shimmer_modifier: 0.8,
            ..Default::default()
        };
        let record = pack(&light, LightPose::default(), PackContext::default());
        let flags = ChannelFlags::unpack(record.channel);
        assert!(flags.realtime);
        assert_eq!(flags.light_type, LightType::Rotor as u32);
        assert_eq!(flags.shimmer, Shimmer::Water as u32);
        assert!((record.shimmer_modifier - 0.2).abs() < 1e-6);
    }

    #[test]
    fn cookies_only_apply_to_spot_lights() {
        let context = PackContext {
            cookie_index: Some(3),
            ..Default::default()
        };
        let point = pack(&DynamicLight::default(), LightPose::default(), context);
        assert!(!ChannelFlags::unpack(point.channel).cookie);
        let spot = DynamicLight {
            light_type: LightType::Spot,
            ..Default::default()
        };
        let record = pack(&spot, LightPose::default(), context);
        assert!(ChannelFlags::unpack(record.channel).cookie);
        assert_eq!(record.cookie_index, 3);
        assert_eq!(record.gp3, cookie_projection_scale(spot.outer_cutoff));
    }

    #[test]
    fn suns_point_towards_the_sun_and_never_fade() {
        let sun = DynamicLight {
            falloff: 1.0,
            ..DynamicLight::sun()
        };
        let record = pack(&sun, pose_looking_down(), PackContext::default());
        assert!(Vec3::from(record.forward).abs_diff_eq(Vec3::Y, 1e-6));
        assert_eq!((record.radius_sqr, record.falloff), (f32::MAX, 0.0));
        for world in [Vec3::ZERO, Vec3::splat(1.0e4), Vec3::new(0.0, 3.0, 0.0)] {
            assert_eq!(mirror::radiance(&record, world), sun.intensity);
        }
    }

    #[test]
    fn bounce_color_defaults_to_the_light_color() {
        let light = DynamicLight {
            color: Color::srgb(1.0, 0.5, 0.25),
            ..Default::default()
        };
        let expected = linear_rgb(light.color);
        let actual = bounce_color(&light);
        for (a, b) in actual.iter().zip(expected) {
            assert!((a - b).abs() < 1e-5);
        }
    }
}
