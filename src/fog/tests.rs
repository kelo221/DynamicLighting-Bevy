use bevy::math::{Quat, Vec2, Vec3};

use super::mirror::{FogVolume, compose, opacity};
use super::*;
use crate::light::{DynamicLight, LightType};

fn volume(shape: VolumetricShape) -> FogVolume {
    FogVolume {
        shape,
        center: Vec3::ZERO,
        axis: Vec3::NEG_Y,
        up: Vec3::Z,
        scale: Vec3::ONE,
        half: Vec2::ZERO,
        slope: shaft_slope(30.0),
        radius: 1.0,
        thickness: 1.0,
        intensity: 0.75,
        inverse_visibility: 0.5,
        cone_term: cone_term(30.0),
    }
}

fn placement() -> Placement {
    Placement {
        position: Vec3::new(1.0, 2.0, 3.0),
        rotation: Quat::from_rotation_x(0.5),
        scale: Vec3::new(1.0, 2.0, 3.0),
    }
}

#[test]
fn defaults_match_henrys_light() {
    let fog = VolumetricLight::default();
    assert_eq!((fog.radius, fog.thickness), (4.0, 1.0));
    assert_eq!((fog.intensity, fog.visibility), (0.75, 2.0));
    assert_eq!(VolumetricShape::ConeY as u32, 4);
}

#[test]
fn inactive_volumes_are_skipped() {
    let light = DynamicLight::default();
    let fog = VolumetricLight::default();
    assert!(is_active(&light, &fog, 1.0));
    assert!(!is_active(&light, &fog, 0.0));
    assert!(!is_active(
        &light,
        &VolumetricLight { radius: 0.0, ..fog },
        1.0
    ));
    assert!(!is_active(
        &light,
        &VolumetricLight {
            intensity: 0.0,
            ..fog
        },
        1.0
    ));
    let broken_spot = DynamicLight {
        light_type: LightType::Spot,
        outer_cutoff: 10.0,
        ..Default::default()
    };
    assert!(!is_active(&broken_spot, &fog, 1.0));
}

#[test]
fn packing_reuses_the_light_record() {
    let light = DynamicLight::default();
    let fog = VolumetricLight {
        shape: VolumetricShape::Box,
        ..Default::default()
    };
    let record = pack(&light, &fog, 0.5, placement());
    assert_eq!(record.channel, 2);
    assert_eq!(record.radius_sqr, 4.0);
    assert_eq!(record.gp1, 1.0);
    assert_eq!(
        (record.gp2, record.gp3, record.shimmer_scale),
        (1.0, 2.0, 3.0)
    );
    assert_eq!(record.intensity, 1.0);
    assert_eq!(record.volumetric_intensity, 0.375);
    assert_eq!(record.volumetric_visibility, 0.5);
    assert_eq!(record.position, [1.0, 2.0, 3.0]);

    let invisible = VolumetricLight {
        visibility: 0.0,
        ..fog
    };
    assert!(pack(&light, &invisible, 1.0, placement()).volumetric_visibility > 1.0e4);
}

#[test]
fn cones_point_along_forward_or_up() {
    let light = DynamicLight::default();
    let rotation = placement().rotation;
    let cone_z = VolumetricLight {
        shape: VolumetricShape::ConeZ,
        ..Default::default()
    };
    let record = pack(&light, &cone_z, 1.0, placement());
    assert!(Vec3::from(record.forward).abs_diff_eq(rotation * Vec3::NEG_Z, 1e-6));
    assert_eq!(record.gp2, cone_term(30.0));

    let cone_y = VolumetricLight {
        shape: VolumetricShape::ConeY,
        ..cone_z
    };
    let record = pack(&light, &cone_y, 1.0, placement());
    assert!(Vec3::from(record.forward).abs_diff_eq(rotation * Vec3::Y, 1e-6));
}

#[test]
fn cone_term_widens_up_to_ninety_degrees() {
    for cutoff in [-30.0_f32, 0.0, 30.0, 90.0, 120.0] {
        let widen = 1.0 + 0.3 * (cutoff / 90.0).clamp(0.0, 1.0);
        let expected = (core::f32::consts::FRAC_PI_2 + cutoff.to_radians()).cos() * widen;
        assert!((cone_term(cutoff) - expected).abs() < 1e-6, "{cutoff}");
    }
}

#[test]
fn bounding_radius_contains_each_shape() {
    let fog = VolumetricLight::default();
    let light = DynamicLight::default();
    assert_eq!(light.outer_cutoff, 30.0);
    assert_eq!(bounding_radius(&fog, &light, Vec3::splat(9.0)), 4.0);
    let fog_box = VolumetricLight {
        shape: VolumetricShape::Box,
        ..fog
    };
    let radius = bounding_radius(&fog_box, &light, Vec3::new(1.0, -2.0, 3.0));
    assert!((radius - 224.0_f32.sqrt()).abs() < 1e-5);
    let cone = VolumetricLight {
        shape: VolumetricShape::ConeZ,
        ..fog
    };
    // The shader's cone opens to |cone_term| radians: 0.55 at 30°, wider than the cutoff.
    let radius = bounding_radius(&cone, &light, Vec3::ONE);
    assert!((radius - 4.0 / 0.55_f32.cos()).abs() < 1e-4, "{radius}");
    let wide_light = DynamicLight {
        outer_cutoff: 147.5,
        ..light.clone()
    };
    let wide = bounding_radius(&cone, &wide_light, Vec3::ONE);
    let angle = 147.5_f32.to_radians().sin() * 1.3;
    assert!((wide - 4.0 / angle.cos()).abs() < 1e-4, "{wide}");
    // A shaft's farthest corners: 4 m along, half extents grown by 4 · tan 30° on each side.
    let shaft = VolumetricLight {
        shape: VolumetricShape::Frustum,
        ..fog
    };
    let panel = DynamicLight {
        outer_cutoff: 30.0,
        ..DynamicLight::rectangle(1.0, 0.5)
    };
    let far = Vec2::new(0.5, 0.25) + 4.0 * 30.0_f32.to_radians().tan();
    let radius = bounding_radius(&shaft, &panel, Vec3::ONE);
    assert!((radius - far.extend(4.0).length()).abs() < 1e-5);
}

#[test]
fn shafts_start_at_the_light_shape_and_widen() {
    let panel = DynamicLight {
        outer_cutoff: 20.0,
        ..DynamicLight::rectangle(1.2, 0.4)
    };
    let fog = VolumetricLight {
        shape: VolumetricShape::Frustum,
        ..Default::default()
    };
    let record = pack(&panel, &fog, 1.0, placement());
    assert_eq!(record.channel, 5);
    assert_eq!((record.gp2, record.gp3), (0.6, 0.2));
    assert_eq!(record.shimmer_scale, shaft_slope(20.0));
    let rotation = placement().rotation;
    assert!(Vec3::from(record.forward).abs_diff_eq(rotation * Vec3::NEG_Z, 1e-6));
    // Point lights start the shaft at a point; the opening stops at 75°.
    let pyramid = pack(&DynamicLight::default(), &fog, 1.0, placement());
    assert_eq!((pyramid.gp2, pyramid.gp3), (0.0, 0.0));
    assert_eq!(shaft_slope(89.0), 75.0_f32.to_radians().tan());
    assert_eq!(shaft_slope(-5.0), 0.0);
}

#[test]
fn shaft_fog_is_square_and_below_the_light() {
    // A 1 m square shaft pointing down 4 m, not widening; up is +Z, so it spans x and z ±0.5.
    let shaft = FogVolume {
        radius: 4.0,
        half: Vec2::splat(0.5),
        slope: 0.0,
        ..volume(VolumetricShape::Frustum)
    };
    let across = |y: f32, z: f32| opacity(&shaft, Vec3::new(-5.0, y, z), Vec3::new(5.0, y, z));
    assert!(across(-2.0, 0.45) > 0.0, "inside the square");
    assert_eq!(across(-2.0, 0.55), 0.0, "outside the square");
    assert_eq!(across(1.0, 0.0), 0.0, "behind the light");
    assert_eq!(across(-4.5, 0.0), 0.0, "past the end");
    // Down the axis the whole length is fog: coverage 1.
    let thick = FogVolume {
        thickness: 10.0,
        inverse_visibility: 10.0,
        ..shaft
    };
    let down = opacity(&thick, Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, -6.0, 0.0));
    assert!((down - 0.75).abs() < 1e-6, "{down}");
    // A widening shaft reaches further out the further it goes.
    let widening = FogVolume {
        slope: 0.5,
        ..shaft
    };
    let wide = |y: f32| opacity(&widening, Vec3::new(-5.0, y, 1.2), Vec3::new(5.0, y, 1.2));
    assert_eq!(wide(-1.0), 0.0);
    assert!(wide(-3.0) > 0.0);
}

#[test]
fn boxes_turn_with_the_light() {
    // Turned 45° about Y, the unit box's corners reach √2 along X and Z.
    let rotation = Quat::from_rotation_y(core::f32::consts::FRAC_PI_4);
    let turned = FogVolume {
        axis: rotation * Vec3::NEG_Z,
        up: Vec3::Y,
        ..volume(VolumetricShape::Box)
    };
    let along_x = |z: f32| opacity(&turned, Vec3::new(-5.0, 0.0, z), Vec3::new(5.0, 0.0, z));
    assert!(along_x(1.2) > 0.0, "a corner pokes out past z = 1");
    assert_eq!(along_x(1.5), 0.0);
    let straight = volume(VolumetricShape::Box);
    let straight_x = opacity(
        &straight,
        Vec3::new(-5.0, 0.0, 1.2),
        Vec3::new(5.0, 0.0, 1.2),
    );
    assert_eq!(straight_x, 0.0);
}

#[test]
fn a_ray_through_the_sphere_center_is_fully_covered() {
    let sphere = volume(VolumetricShape::Sphere);
    let through = opacity(&sphere, Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0));
    assert!((through - 0.75).abs() < 1e-6);
    let miss = opacity(&sphere, Vec3::new(-5.0, 2.0, 0.0), Vec3::new(5.0, 2.0, 0.0));
    assert_eq!(miss, 0.0);
    let grazing = opacity(&sphere, Vec3::new(-5.0, 0.5, 0.0), Vec3::new(5.0, 0.5, 0.0));
    assert!(grazing > 0.0 && grazing < through);
}

#[test]
fn nearby_surfaces_stay_visible_inside_fog() {
    let sphere = volume(VolumetricShape::Sphere);
    let near = opacity(&sphere, Vec3::ZERO, Vec3::new(0.2, 0.0, 0.0));
    assert!((near - 0.2 * 0.5 * 0.75).abs() < 1e-6);
}

#[test]
fn box_coverage_is_the_path_length_over_the_diagonal() {
    let fog_box = FogVolume {
        thickness: 10.0,
        inverse_visibility: 10.0,
        ..volume(VolumetricShape::Box)
    };
    let through = opacity(
        &fog_box,
        Vec3::new(-5.0, 0.0, 0.0),
        Vec3::new(5.0, 0.0, 0.0),
    );
    assert!(
        (through - 0.75).abs() < 1e-6,
        "thick box saturates: {through}"
    );
    let thin = FogVolume {
        thickness: 1.0,
        ..fog_box
    };
    let coverage = 2.0 / 12.0_f32.sqrt();
    let expected = coverage * coverage * (3.0 - 2.0 * coverage) * 0.75;
    let actual = opacity(&thin, Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0));
    assert!((actual - expected).abs() < 1e-5);
    assert_eq!(
        opacity(&thin, Vec3::new(-5.0, 3.0, 0.0), Vec3::new(5.0, 3.0, 0.0)),
        0.0
    );
}

#[test]
fn cone_fog_lies_along_its_axis() {
    let cone = FogVolume {
        radius: 4.0,
        ..volume(VolumetricShape::ConeY)
    };
    let across = opacity(&cone, Vec3::new(-5.0, -3.0, 0.0), Vec3::new(5.0, -3.0, 0.0));
    assert!(across > 0.0);
    let above = opacity(&cone, Vec3::new(-5.0, 3.0, 0.0), Vec3::new(5.0, 3.0, 0.0));
    assert_eq!(above, 0.0);
}

#[test]
fn compose_blends_towards_opaque_fog() {
    let scene = Vec3::new(0.2, 0.4, 0.6);
    assert!(compose(scene, &[]).abs_diff_eq(scene, 1e-6));
    let solid = compose(scene, &[(Vec3::new(1.0, 0.0, 0.0), 1.0)]);
    assert!(solid.abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-6));
    // screen(0.25, s) lerped towards 0.25 by 0.25 is 0.25 + 0.5625·s: light fog lifts dark
    // pixels and slightly dims bright ones, as in Henry's post-process.
    let light_fog = compose(scene, &[(Vec3::ONE, 0.25)]);
    assert!(light_fog.abs_diff_eq(Vec3::splat(0.25) + scene * 0.5625, 1e-6));
    assert!(light_fog.x > scene.x && light_fog.z < scene.z);
}
