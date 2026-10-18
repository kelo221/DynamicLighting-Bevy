use bevy::math::{Dir3, Vec3};
use bevy::prelude::Transform;

use super::mirror::{attenuation, radiance, to_light_space, type_factor};
use super::*;
use crate::shared::{ChannelFlags, GpuLight};

fn gpu(light_type: LightType, transform: Transform, gp: [f32; 3]) -> GpuLight {
    let forward = if light_type == LightType::Spot {
        transform.back()
    } else {
        transform.forward()
    };
    GpuLight {
        position: transform.translation.to_array(),
        radius_sqr: 16.0,
        channel: ChannelFlags {
            light_type: light_type as u32,
            ..Default::default()
        }
        .pack(),
        intensity: 1.0,
        gp1: gp[0],
        gp2: gp[1],
        gp3: gp[2],
        up: transform.up().to_array(),
        forward: forward.to_array(),
        ..Default::default()
    }
}

/// Points on concentric spheres around the origin, inside a 4 m radius.
fn sample_points() -> impl Iterator<Item = Vec3> {
    [0.5, 1.3, 2.1, 2.9, 3.7].into_iter().flat_map(|r| {
        (0..40).flat_map(move |i| {
            (0..40).map(move |j| {
                let theta = i as f32 / 40.0 * core::f32::consts::PI;
                let phi = j as f32 / 40.0 * core::f32::consts::TAU;
                Vec3::new(
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                ) * r
            })
        })
    })
}

#[test]
fn defaults_match_henrys_light() {
    let light = DynamicLight::default();
    assert_eq!(
        (light.intensity, light.radius, light.falloff),
        (2.0, 4.0, 0.0)
    );
    assert_eq!((light.inner_cutoff, light.outer_cutoff), (26.0, 30.0));
    assert_eq!((light.rotor_center, light.shimmer_scale), (0.1, 12.25));
    assert_eq!(light.shimmer_modifier, 0.8);
    assert_eq!(light.bounce.samples, 32);
    assert_eq!(light.shadows, ShadowMode::Raytraced);
    assert!(!light.realtime);
}

#[test]
fn light_type_bits_round_trip() {
    for light_type in LightType::ALL {
        assert_eq!(LightType::from_bits(light_type as u32), Some(light_type));
    }
    assert_eq!(LightType::from_bits(LightType::ALL.len() as u32), None);
}

#[test]
fn attenuation_falls_smoothly_to_zero_at_the_radius() {
    let light = gpu(LightType::Point, Transform::IDENTITY, [0.0; 3]);
    assert_eq!(attenuation(&light, 0.0), 1.0);
    assert_eq!(attenuation(&light, 16.0), 0.0);
    let samples: Vec<f32> = (0..=16).map(|d| attenuation(&light, d as f32)).collect();
    assert!(samples.windows(2).all(|pair| pair[1] < pair[0]));

    let with_falloff = GpuLight {
        falloff: 4.0,
        ..light
    };
    assert!(attenuation(&with_falloff, 4.0) < attenuation(&light, 4.0));
}

#[test]
fn radiance_is_zero_outside_the_radius_and_for_disabled_slots() {
    let light = gpu(LightType::Point, Transform::IDENTITY, [0.0; 3]);
    assert!(radiance(&light, Vec3::X) > 0.0);
    assert_eq!(radiance(&light, Vec3::X * 4.5), 0.0);
    assert_eq!(radiance(&GpuLight::DISABLED, Vec3::X), 0.0);
}

#[test]
fn spot_lights_shine_along_their_forward_axis() {
    let down = Transform::from_xyz(0.0, 3.0, 0.0).looking_to(Dir3::NEG_Y, Vec3::Z);
    let cosines = [
        26.0_f32.to_radians().cos(),
        30.0_f32.to_radians().cos(),
        0.0,
    ];
    let spot = gpu(LightType::Spot, down, cosines);
    let below = Vec3::new(0.0, 1.0, 0.0);
    assert_eq!(type_factor(&spot, below), 1.0);
    assert_eq!(type_factor(&spot, Vec3::new(0.0, 5.0, 0.0)), 0.0, "behind");
    assert_eq!(type_factor(&spot, Vec3::new(3.0, 2.5, 0.0)), 0.0, "outside");
    let edge = Vec3::new(2.0 * 28.0_f32.to_radians().tan(), 1.0, 0.0);
    let soft = type_factor(&spot, edge);
    assert!(soft > 0.0 && soft < 1.0, "between the cutoffs: {soft}");
}

#[test]
fn equal_spot_cutoffs_give_a_hard_edge() {
    let cosine = 30.0_f32.to_radians().cos();
    let spot = gpu(LightType::Spot, Transform::IDENTITY, [cosine, cosine, 0.0]);
    assert_eq!(type_factor(&spot, Vec3::new(0.0, 0.0, -2.0)), 1.0);
    assert_eq!(type_factor(&spot, Vec3::new(2.0, 0.0, -1.0)), 0.0);
}

#[test]
fn discoball_lights_axis_and_diagonal_directions() {
    let cosines = [5.0_f32.to_radians().cos(), 8.0_f32.to_radians().cos(), 0.0];
    let ball = gpu(LightType::Discoball, Transform::IDENTITY, cosines);
    for world in [Vec3::X, Vec3::NEG_Y, Vec3::new(1.0, 1.0, 0.0), Vec3::ONE] {
        assert_eq!(type_factor(&ball, world * 2.0), 1.0, "{world}");
    }
    let between = Vec3::new(1.0, 15.0_f32.to_radians().tan(), 0.0).normalize();
    assert_eq!(type_factor(&ball, between * 2.0), 0.0);
}

#[test]
fn patterned_types_stay_in_their_ranges() {
    let tilted = Transform::from_rotation(bevy::math::Quat::from_euler(
        bevy::math::EulerRot::YXZ,
        0.6,
        -0.4,
        0.2,
    ));
    let cases = [
        (LightType::Wave, [0.3, 2.0, 0.0], 0.4, 1.0),
        (LightType::Interference, [0.5, 3.0, 0.0], 0.0, 1.0),
        (LightType::Rotor, [0.5, 3.0, 0.1], 0.0, 1.0),
        (LightType::Rotor, [0.5, 3.0, -0.2], 0.0, 1.0),
        (LightType::Shock, [0.3, 1.0, 0.0], 0.512, 1.0),
        (LightType::Disco, [0.5, 4.0, 1.0], 0.0, 1.0),
    ];
    for (light_type, gp, low, high) in cases {
        let light = gpu(light_type, tilted, gp);
        let values: Vec<f32> = sample_points().map(|w| type_factor(&light, w)).collect();
        for value in &values {
            assert!(
                value.is_finite() && (low - 1e-5..=high + 1e-5).contains(value),
                "{light_type:?}: {value}"
            );
        }
        let lowest = values.iter().fold(f32::MAX, |a, &b| a.min(b));
        assert!(lowest < high - 0.05, "{light_type:?} pattern is flat");
    }
}

#[test]
fn rotor_center_blob_is_light_or_shadow() {
    let light = gpu(LightType::Rotor, Transform::IDENTITY, [0.0, 3.0, 0.1]);
    assert_eq!(type_factor(&light, Vec3::new(0.0, -2.0, 0.0)), 1.0);
    let shadow = gpu(LightType::Rotor, Transform::IDENTITY, [0.0, 3.0, -0.1]);
    assert_eq!(type_factor(&shadow, Vec3::new(0.0, -2.0, 0.0)), 0.0);
}

#[test]
fn light_space_right_axis_is_mirrored_for_bevy() {
    let identity = Transform::IDENTITY;
    let local = to_light_space(Vec3::NEG_X, *identity.forward(), *identity.up());
    assert_eq!(local, Vec3::new(1.0, 0.0, 0.0));
    let local = to_light_space(Vec3::NEG_Z, *identity.forward(), *identity.up());
    assert_eq!(local, Vec3::new(0.0, 0.0, 1.0));
}
