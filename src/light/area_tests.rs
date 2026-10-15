//! The area-light math against brute-force integration over the shapes.

use bevy::math::{Quat, Vec2, Vec3};
use bevy::transform::components::Transform;

use super::area::{self, AreaShape};
use super::{DynamicLight, LightPose, PackContext, mirror, pack};
use crate::shared::GpuLight;

/// A 2 × 1 panel at the origin facing down, spanning x ∈ [-1, 1] and z ∈ [-0.5, 0.5].
fn panel() -> AreaShape {
    AreaShape {
        center: Vec3::ZERO,
        x: Vec3::X,
        y: Vec3::NEG_Z,
        forward: Vec3::NEG_Y,
        half: Vec2::new(1.0, 0.5),
        rectangle: true,
    }
}

/// A tube from x = -1 to x = 1.
fn tube() -> AreaShape {
    AreaShape {
        half: Vec2::new(1.0, 0.0),
        rectangle: false,
        ..panel()
    }
}

/// Mean cosine over the panel by summing small patches, each weighted by its solid angle.
fn brute_rectangle(shape: &AreaShape, world: Vec3, normal: Vec3, clip: bool) -> f32 {
    let n = 400;
    let (mut weighted, mut total) = (0.0f64, 0.0f64);
    for i in 0..n {
        for j in 0..n {
            let u = ((i as f32 + 0.5) / n as f32 * 2.0 - 1.0) * shape.half.x;
            let v = ((j as f32 + 0.5) / n as f32 * 2.0 - 1.0) * shape.half.y;
            let to = shape.center + shape.x * u + shape.y * v - world;
            let distance = to.length();
            let direction = to / distance;
            let solid = (shape.forward.dot(direction).abs() / (distance * distance)) as f64;
            let cosine = normal.dot(direction);
            weighted += solid * if clip { cosine.max(0.0) } else { cosine } as f64;
            total += solid;
        }
    }
    (weighted / total) as f32
}

/// Mean cosine over the tube by summing short pieces, each weighted by the angle it covers.
fn brute_tube(shape: &AreaShape, world: Vec3, normal: Vec3, clip: bool) -> f32 {
    let n = 20_000;
    let (mut weighted, mut total) = (0.0f64, 0.0f64);
    for i in 0..n {
        let u = ((i as f32 + 0.5) / n as f32 * 2.0 - 1.0) * shape.half.x;
        let to = shape.center + shape.x * u - world;
        let distance = to.length();
        let direction = to / distance;
        let angle = (direction.cross(shape.x).length() / distance) as f64;
        let cosine = normal.dot(direction);
        weighted += angle * if clip { cosine.max(0.0) } else { cosine } as f64;
        total += angle;
    }
    (weighted / total) as f32
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn rectangle_mean_cosine_matches_brute_force() {
    let shape = panel();
    let cases = [
        (Vec3::new(0.0, -2.0, 0.0), Vec3::Y),
        (Vec3::new(3.0, -1.0, 0.5), Vec3::Y),
        (Vec3::new(0.2, -0.3, 0.1), Vec3::Y),
        // A wall beside the panel that sees only part of it above its horizon.
        (Vec3::new(0.5, -0.5, 0.0), Vec3::NEG_X),
        (
            Vec3::new(1.5, -0.4, -0.2),
            Vec3::new(-1.0, 1.0, 0.0).normalize(),
        ),
    ];
    for (world, normal) in cases {
        for clip in [false, true] {
            let expected = brute_rectangle(&shape, world, normal, clip);
            let actual = area::rectangle_cosine(&shape, world, normal, clip);
            assert!(
                close(actual, expected, 2e-3),
                "{world} {normal} clip {clip}: {actual} != {expected}"
            );
        }
    }
}

#[test]
fn tube_mean_cosine_matches_brute_force() {
    let shape = tube();
    let cases = [
        (Vec3::new(0.0, -1.0, 0.0), Vec3::Y),
        (Vec3::new(0.5, -0.3, 0.2), Vec3::NEG_X),
        (Vec3::new(3.0, 0.2, 0.0), Vec3::NEG_X),
        (
            Vec3::new(-2.0, -1.0, 1.0),
            Vec3::new(1.0, 1.0, -1.0).normalize(),
        ),
    ];
    for (world, normal) in cases {
        for clip in [false, true] {
            let expected = brute_tube(&shape, world, normal, clip);
            let actual = area::tube_cosine(&shape, world, normal, clip);
            assert!(
                close(actual, expected, 2e-3),
                "{world} {normal} clip {clip}: {actual} != {expected}"
            );
        }
    }
}

#[test]
fn small_shapes_light_like_points() {
    for shape in [panel(), tube()] {
        let small = AreaShape {
            half: shape.half * 0.02,
            ..shape
        };
        let world = Vec3::new(1.0, -2.5, 0.7);
        let normal = Vec3::new(0.3, 1.0, -0.2).normalize();
        let cosine = normal.dot((small.center - world).normalize());
        for baked in [false, true] {
            let actual = if small.rectangle {
                area::rectangle_cosine(&small, world, normal, baked)
            } else {
                area::tube_cosine(&small, world, normal, baked)
            };
            assert!(close(actual, cosine, 2e-3), "{actual} != {cosine}");
        }
    }
}

#[test]
fn panels_light_only_their_front() {
    let shape = panel();
    let below = area::diffuse(&shape, Vec3::new(0.0, -2.0, 0.0), Vec3::Y, true);
    assert!(below > 0.9 && below < 1.0, "{below}");
    assert_eq!(
        area::diffuse(&shape, Vec3::new(0.0, 2.0, 0.0), Vec3::NEG_Y, true),
        0.0
    );
    assert_eq!(
        area::diffuse(&shape, Vec3::new(0.0, 2.0, 0.0), Vec3::NEG_Y, false),
        0.0,
        "half-Lambert doesn't wrap around a panel's back"
    );
    // Seen from the side, a panel fades with its emission cosine.
    let side = area::diffuse(&shape, Vec3::new(4.0, -1.0, 0.0), Vec3::NEG_X, true);
    assert!(side > 0.0 && side < 0.4, "{side}");
}

#[test]
fn representative_points_follow_the_reflection() {
    let shape = panel();
    let world = Vec3::new(0.0, -2.0, 0.0);
    let at = |reflection: Vec3| area::representative_point(&shape, world, reflection.normalize());
    assert!(at(Vec3::Y).abs_diff_eq(Vec3::ZERO, 1e-5));
    assert!(at(Vec3::new(0.3, 1.0, 0.0)).abs_diff_eq(Vec3::new(0.6, 0.0, 0.0), 1e-5));
    assert!(at(Vec3::new(2.0, 1.0, 0.0)).abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-5));
    assert!(
        at(Vec3::new(0.0, -1.0, 1.0)).y.abs() < 1e-6,
        "rays pointing away stay on it"
    );

    let tube = tube();
    let world = Vec3::new(0.5, -1.0, 0.0);
    let point = area::representative_point(&tube, world, Vec3::Y);
    assert!(point.abs_diff_eq(Vec3::new(0.5, 0.0, 0.0), 1e-5), "{point}");
    let end = area::representative_point(&tube, world, Vec3::new(5.0, 1.0, 0.0).normalize());
    assert!(end.abs_diff_eq(Vec3::new(1.0, 0.0, 0.0), 1e-5), "{end}");
}

#[test]
fn larger_shapes_spread_their_highlight() {
    let world = Vec3::new(0.0, -2.0, 0.0);
    let scale = |half: Vec2, rectangle: bool| {
        let shape = AreaShape {
            half,
            rectangle,
            ..panel()
        };
        area::specular_scale(&shape, world, Vec3::ZERO, 0.3)
    };
    assert!(close(scale(Vec2::ZERO, true), 1.0, 1e-6));
    assert!(close(scale(Vec2::ZERO, false), 1.0, 1e-6));
    assert!(scale(Vec2::new(1.0, 0.5), true) < scale(Vec2::new(0.5, 0.5), true));
    assert!(scale(Vec2::new(1.0, 0.0), false) > scale(Vec2::new(1.0, 1.0), true));
    let behind = area::specular_scale(&panel(), Vec3::new(0.0, 2.0, 0.0), Vec3::ZERO, 0.3);
    assert_eq!(behind, 0.0);
}

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
