//! The area-light math against brute-force integration over the shapes.

use bevy::math::{Vec2, Vec3};

use super::area::{self, AreaShape};

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

/// The largest cosine between `normal` and the directions to a grid of points on the shape.
fn brute_facing(shape: &AreaShape, world: Vec3, normal: Vec3) -> f32 {
    let n = 400;
    let rows = if shape.rectangle { n } else { 1 };
    let mut best = -1.0f32;
    for i in 0..=n {
        for j in 0..=rows {
            let u = (i as f32 / n as f32 * 2.0 - 1.0) * shape.half.x;
            let v = (j as f32 / rows.max(1) as f32 * 2.0 - 1.0) * shape.half.y;
            let to = shape.center + shape.x * u + shape.y * v - world;
            best = best.max(normal.dot(to.normalize()));
        }
    }
    best
}

#[test]
fn the_facing_cosine_is_the_best_cosine_over_the_shape() {
    let cases = [
        (Vec3::new(0.0, -2.0, 0.0), Vec3::Y),
        (
            Vec3::new(0.3, -0.5, 0.1),
            Vec3::new(0.2, 1.0, 0.1).normalize(),
        ),
        // A wall beside the panel that sees only part of it above its horizon.
        (Vec3::new(0.5, -0.5, 0.0), Vec3::NEG_X),
        (
            Vec3::new(1.5, -0.4, -0.2),
            Vec3::new(-1.0, 0.3, 0.0).normalize(),
        ),
        (
            Vec3::new(-2.0, -1.0, 1.0),
            Vec3::new(1.0, 0.2, -1.0).normalize(),
        ),
        // Turned away from all of it.
        (
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::new(0.3, -1.0, 0.0).normalize(),
        ),
    ];
    for shape in [panel(), tube()] {
        for (world, normal) in cases {
            let expected = brute_facing(&shape, world, normal);
            let actual = area::facing_cosine(&shape, world, normal);
            assert!(
                close(actual, expected, 2e-3),
                "{} {world} {normal}: {actual} != {expected}",
                shape.rectangle
            );
            let clipped = if shape.rectangle {
                area::rectangle_cosine(&shape, world, normal, true)
            } else {
                area::tube_cosine(&shape, world, normal, true)
            };
            assert_eq!(actual > 1e-4, clipped > 1e-6, "{world} {normal}");
        }
    }
}

#[test]
fn surfaces_facing_the_end_of_a_shape_face_it() {
    // Beside the tube's far end, facing along it: the center is behind the horizon, the end
    // well above it.
    let world = Vec3::new(1.2, -0.3, 0.0);
    let normal = Vec3::X;
    let shape = AreaShape {
        center: Vec3::new(-1.0, 0.0, 0.0),
        half: Vec2::new(2.5, 0.0),
        ..tube()
    };
    assert!(normal.dot((shape.center - world).normalize()) < -0.9);
    let facing = area::facing_cosine(&shape, world, normal);
    assert!(facing > 0.5, "{facing}");
    // Small shapes face like points.
    let small = AreaShape {
        half: Vec2::new(0.01, 0.005),
        ..panel()
    };
    let world = Vec3::new(1.0, -2.5, 0.7);
    let normal = Vec3::new(-0.3, 1.0, -0.2).normalize();
    let cosine = normal.dot((small.center - world).normalize());
    for shape in [
        small,
        AreaShape {
            rectangle: false,
            ..small
        },
    ] {
        let facing = area::facing_cosine(&shape, world, normal);
        assert!(close(facing, cosine, 2e-3), "{facing} != {cosine}");
    }
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
