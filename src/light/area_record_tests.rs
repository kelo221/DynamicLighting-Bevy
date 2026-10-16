//! Area-light records: the shape packed into `GpuLight`, the radius around the position, and the
//! records of shadow points lighting like the whole light.

use bevy::math::{Quat, Vec2, Vec3};
use bevy::transform::components::Transform;

use super::area::AreaShape;
use super::{DynamicLight, LightPose, PackContext, mirror, pack};
use crate::shared::GpuLight;

fn packed(light: DynamicLight, transform: Transform) -> GpuLight {
    let pose = LightPose {
        position: transform.translation,
        rotation: transform.rotation,
    };
    pack(&light, pose, PackContext::default())
}

#[test]
fn packed_lights_carry_their_shape() {
    let down = Transform::from_xyz(0.0, 3.0, 0.0).looking_to(Vec3::NEG_Y, Vec3::NEG_Z);
    let record = packed(DynamicLight::rectangle(2.0, 1.0), down);
    let shape = AreaShape::of(&record).expect("a rectangle");
    assert!(shape.x.abs_diff_eq(Vec3::X, 1e-6));
    assert!(shape.y.abs_diff_eq(Vec3::NEG_Z, 1e-6));
    assert!(shape.forward.abs_diff_eq(Vec3::NEG_Y, 1e-6));
    assert_eq!(shape.half, Vec2::new(1.0, 0.5));
    assert!(shape.rectangle);

    let turned = Transform::from_rotation(Quat::from_rotation_y(core::f32::consts::FRAC_PI_2));
    let tube = AreaShape::of(&packed(DynamicLight::tube(3.0), turned)).expect("a tube");
    assert!(tube.x.abs_diff_eq(Vec3::NEG_Z, 1e-6), "{}", tube.x);
    assert_eq!(tube.half, Vec2::new(1.5, 0.0));
    assert!(AreaShape::of(&packed(DynamicLight::point(), down)).is_none());
}

#[test]
fn area_lights_end_within_their_radius() {
    let light = DynamicLight {
        radius: 4.0,
        ..DynamicLight::tube(4.0)
    };
    assert_eq!(light.shape_reach(), 2.0);
    let record = packed(light, Transform::IDENTITY);
    assert_eq!(record.radius_sqr, 4.0);
    // 1.5 m off the end of the tube, 3.5 m from its center: lit from the nearest point.
    let world = Vec3::new(3.5, 0.0, 0.0);
    assert!(mirror::source_point(&record, world).abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-6));
    assert!(mirror::radiance(&record, world) > 0.0);
    // Like every light, nothing past the radius around the position is lit, so the bake's
    // sphere around the position holds everything the light reaches.
    let panel = packed(
        DynamicLight {
            radius: 3.0,
            ..DynamicLight::rectangle(2.0, 1.0)
        },
        Transform::from_rotation(Quat::from_rotation_x(0.7)),
    );
    for record in [record, panel] {
        let radius = record.radius_sqr.sqrt() + AreaShape::of(&record).unwrap().half.length();
        for i in 0..2000 {
            let t = i as f32;
            let direction = Vec3::new((t * 0.37).sin(), (t * 0.61).cos(), (t * 0.13).sin());
            let world = direction.normalize() * (radius + 0.01 + (t * 0.07).fract());
            assert_eq!(mirror::radiance(&record, world), 0.0, "{world}");
        }
    }
}

#[test]
fn area_lights_need_a_radius_beyond_their_shape() {
    assert!(!DynamicLight::rectangle(-1.0, 1.0).is_valid());
    assert!(DynamicLight::rectangle(2.0, 1.0).is_valid());
    let enclosed = DynamicLight {
        radius: 1.0,
        ..DynamicLight::rectangle(2.0, 1.0)
    };
    assert!(!enclosed.is_valid(), "the panel reaches past the radius");
    assert_eq!(
        packed(enclosed, Transform::IDENTITY).radius_sqr,
        GpuLight::DISABLED.radius_sqr
    );
}

#[test]
fn shadow_point_records_keep_the_shape_where_the_light_is() {
    let transform = Transform::from_xyz(1.0, 3.0, -2.0).with_rotation(Quat::from_rotation_y(0.8));
    let pose = LightPose {
        position: transform.translation,
        rotation: transform.rotation,
    };
    for light in [
        DynamicLight::tube(2.0),
        DynamicLight::rectangle(1.6, 0.4),
        DynamicLight::rectangle(0.4, 1.6),
    ] {
        let whole = pack(&light, pose, PackContext::default());
        let points: Vec<_> = light
            .shadow_point_layout(pose.position, pose.rotation)
            .collect();
        assert_eq!(points.len(), 3);
        let records: Vec<GpuLight> = points
            .iter()
            .map(|point| {
                let context = PackContext {
                    point_offset: point.offset,
                    share: 1.0 / 3.0,
                    ..Default::default()
                };
                pack(&light, pose, context)
            })
            .collect();
        for (point, record) in points.iter().zip(&records) {
            assert!(Vec3::from(record.position).abs_diff_eq(point.position, 1e-5));
            let shape = AreaShape::of(record).unwrap();
            assert!(shape.center.abs_diff_eq(pose.position, 1e-5), "{light:?}");
        }
        // Together the records light the scene like the whole light.
        for world in [Vec3::new(1.5, 0.0, -1.0), Vec3::new(-0.5, 1.0, -3.0)] {
            let sum: f32 = records.iter().map(|r| mirror::radiance(r, world)).sum();
            let expected = mirror::radiance(&whole, world);
            assert!(
                (sum - expected).abs() < 1e-4 * expected.max(1.0),
                "{sum} {expected}"
            );
        }
    }
}
