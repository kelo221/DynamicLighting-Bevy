//! The LTC table and integral against Lambert's form factor and brute-force GGX integration.

use core::f32::consts::PI;

use bevy::math::{Quat, Vec2, Vec3, Vec4};

use super::area::{self, AreaShape};
use super::ltc::{self, IDENTITY, LUT_BYTES, LUT_SIZE};

/// A rectangle at `center` facing `forward`, `size` wide and high.
fn panel(center: Vec3, forward: Vec3, size: Vec2) -> AreaShape {
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, forward.normalize());
    AreaShape {
        center,
        x: rotation * Vec3::X,
        y: rotation * Vec3::Y,
        forward: forward.normalize(),
        half: size * 0.5,
        rectangle: true,
    }
}

/// Smith's height-correlated masking-shadowing for GGX, the BRDF the table was fitted to.
fn lambda(alpha: f32, cosine: f32) -> f32 {
    if cosine >= 1.0 {
        return 0.0;
    }
    let tangent = (1.0 - cosine * cosine).max(0.0).sqrt() / cosine;
    let a = 1.0 / (alpha * tangent);
    0.5 * (-1.0 + (1.0 + 1.0 / (a * a)).sqrt())
}

/// The fitted BRDF times the light's cosine, without Fresnel.
fn ggx_cosine(normal: Vec3, view: Vec3, light: Vec3, roughness: f32) -> f32 {
    let alpha = roughness * roughness;
    let (n_dot_v, n_dot_l) = (normal.dot(view), normal.dot(light));
    if n_dot_v <= 0.0 || n_dot_l <= 0.0 {
        return 0.0;
    }
    let h = (view + light).normalize();
    let n_dot_h = normal.dot(h);
    let d = alpha * alpha / (PI * (n_dot_h * n_dot_h * (alpha * alpha - 1.0) + 1.0).powi(2));
    let g = 1.0 / (1.0 + lambda(alpha, n_dot_v) + lambda(alpha, n_dot_l));
    d * g / (4.0 * n_dot_v)
}

/// `∫ f cos dω` over the rectangle and its solid angle, by a midpoint grid of `n × n` cells.
fn brute(shape: &AreaShape, world: Vec3, normal: Vec3, view: Vec3, roughness: f32) -> (f32, f32) {
    let n = 400;
    let cell = shape.half * 2.0 / n as f32;
    let (mut sum, mut solid) = (0.0, 0.0);
    for i in 0..n {
        for j in 0..n {
            let uv = Vec2::new((i as f32 + 0.5) / n as f32, (j as f32 + 0.5) / n as f32);
            let to = shape.point_at(uv) - world;
            let d2 = to.length_squared();
            let light = to / d2.sqrt();
            let d_omega = cell.x * cell.y * shape.forward.dot(-light).max(0.0) / d2;
            sum += ggx_cosine(normal, view, light, roughness) * d_omega;
            solid += d_omega;
        }
    }
    (sum, solid)
}

fn view_at(angle_degrees: f32) -> Vec3 {
    let angle = angle_degrees.to_radians();
    Vec3::new(angle.sin(), angle.cos(), 0.0)
}

#[test]
fn the_table_is_the_fit() {
    assert_eq!(LUT_BYTES.len(), (2 * LUT_SIZE * LUT_SIZE * 8) as usize);
    // Smooth surfaces seen head-on: a sharp, isotropic lobe that reflects everything.
    let smooth = ltc::texel(0, 0, 0);
    assert!(smooth.truncate().abs_diff_eq(Vec3::X, 1.0e-3) && smooth.w < 1.0e-3);
    assert!(ltc::texel(1, 0, 0).abs_diff_eq(Vec4::new(1.0, 0.0, 0.0, 0.0), 1.0e-3));
    for y in 0..LUT_SIZE {
        for x in 0..LUT_SIZE {
            let albedo = ltc::texel(1, x, y);
            assert!((0.0..=1.0).contains(&albedo.x), "{albedo} at {x}, {y}");
            assert!(albedo.y <= albedo.x + 1.0e-3, "{albedo} at {x}, {y}");
        }
    }
}

#[test]
fn lookups_land_on_texel_centers() {
    assert_eq!(
        ltc::sample_layer(0, ltc::lut_uv(0.0, 1.0)),
        ltc::texel(0, 0, 0)
    );
    assert_eq!(
        ltc::sample_layer(1, ltc::lut_uv(1.0, 0.0)),
        ltc::texel(1, 63, 63)
    );
    let between = ltc::sample_layer(0, ltc::lut_uv(0.5 / 63.0, 1.0));
    let mean = (ltc::texel(0, 0, 0) + ltc::texel(0, 1, 0)) * 0.5;
    assert!(between.abs_diff_eq(mean, 1.0e-6), "{between} vs {mean}");
}

#[test]
fn the_identity_transform_gives_lamberts_form_factor() {
    let cases = [
        (Vec3::new(0.0, 2.0, 0.0), Vec3::NEG_Y, Vec3::Y),
        (
            Vec3::new(1.5, 1.0, -0.5),
            Vec3::new(-1.0, -1.0, 0.2),
            Vec3::Y,
        ),
        // Partly below the horizon, so the polygon is clipped.
        (Vec3::new(2.0, 0.2, 0.0), Vec3::NEG_X, Vec3::Y),
        (
            Vec3::new(0.3, 0.8, 0.0),
            Vec3::NEG_Y,
            Vec3::new(0.8, 0.6, 0.0),
        ),
    ];
    for (center, forward, normal) in cases {
        let shape = panel(center, forward, Vec2::new(1.6, 0.9));
        let corners = shape.corners().map(|corner| corner - Vec3::ZERO);
        let ltc = ltc::quad_integral(corners, normal, view_at(30.0), IDENTITY);
        let solid = area::solid_angle(&shape, Vec3::ZERO);
        let lambert = area::rectangle_cosine(&shape, Vec3::ZERO, normal, true) * solid / PI;
        let error = (ltc - lambert).abs() / lambert;
        println!("{center} identity {ltc} lambert {lambert} relative {error}");
        assert!(error < 0.005, "{center}: {ltc} vs {lambert}");
    }
}

#[test]
fn the_fit_matches_integrated_ggx() {
    let mut worst = 0.0f32;
    for (roughness, angle) in [(0.3, 20.0), (0.5, 45.0), (0.7, 60.0), (0.9, 30.0)] {
        for center in [Vec3::new(-1.0, 1.5, 0.0), Vec3::new(0.5, 1.2, 0.7)] {
            let shape = panel(center, Vec3::new(0.2, -1.0, 0.1), Vec2::new(1.2, 0.8));
            let view = view_at(angle);
            let table = ltc::lookup(roughness, view.y);
            let corners = shape.corners();
            let ltc = table.albedo.x * ltc::quad_integral(corners, Vec3::Y, view, table.matrix);
            let (reference, _) = brute(&shape, Vec3::ZERO, Vec3::Y, view, roughness);
            // The fit is closest inside the highlight and loosest in the lobe's tail.
            let error = (ltc - reference).abs() / (reference + 0.1);
            worst = worst.max(error);
        }
    }
    assert!(worst < 0.1, "{worst}");
}

/// The transformed lobe integrated over the rectangle by brute force, for a surface facing +Y
/// seen along `view` in the XY plane.
fn brute_lobe(shape: &AreaShape, view: Vec3, matrix: Vec4) -> f32 {
    let frame = if view.x.abs() > 1.0e-6 {
        [
            Vec3::new(view.x.signum(), 0.0, 0.0),
            Vec3::new(0.0, 0.0, -view.x.signum()),
            Vec3::Y,
        ]
    } else {
        [Vec3::X, Vec3::NEG_Z, Vec3::Y]
    };
    let det = (matrix.x * matrix.w - matrix.z * matrix.y).abs();
    let n = 400;
    let cell = shape.half * 2.0 / n as f32;
    let mut sum = 0.0;
    for i in 0..n {
        for j in 0..n {
            let uv = Vec2::new((i as f32 + 0.5) / n as f32, (j as f32 + 0.5) / n as f32);
            let to = shape.point_at(uv);
            let d2 = to.length_squared();
            let direction = to / d2.sqrt();
            let d_omega = cell.x * cell.y * shape.forward.dot(-direction).max(0.0) / d2;
            let local = Vec3::new(
                direction.dot(frame[0]),
                direction.dot(frame[1]),
                direction.dot(frame[2]),
            );
            let w = Vec3::new(
                matrix.x * local.x + matrix.z * local.z,
                local.y,
                matrix.y * local.x + matrix.w * local.z,
            );
            let length = w.length();
            let cosine = (w.z / length).max(0.0) / PI;
            sum += cosine * det / (length * length * length) * d_omega;
        }
    }
    sum
}

#[test]
fn the_polygon_integral_is_the_transformed_lobe_over_the_rectangle() {
    for (roughness, angle) in [(0.3, 20.0), (0.5, 45.0), (0.7, 70.0), (0.6, 0.0)] {
        // Above, beside and partly below the surface's horizon.
        for center in [
            Vec3::new(-1.0, 1.5, 0.0),
            Vec3::new(0.5, 1.2, 0.7),
            Vec3::new(-1.5, 0.2, 0.3),
        ] {
            let shape = panel(center, Vec3::new(0.6, -1.0, 0.1), Vec2::new(1.2, 0.8));
            let view = view_at(angle);
            let matrix = ltc::lookup(roughness, view.y).matrix;
            let ltc = ltc::quad_integral(shape.corners(), Vec3::Y, view, matrix);
            let reference = brute_lobe(&shape, view, matrix);
            assert!(
                (ltc - reference).abs() < 0.01 * reference + 1.0e-4,
                "r {roughness} view {angle} at {center}: {ltc} vs {reference}"
            );
        }
    }
}

#[test]
fn only_the_front_of_a_rectangle_reflects() {
    let shape = panel(Vec3::new(0.0, 2.0, 0.0), Vec3::NEG_Y, Vec2::new(1.0, 1.0));
    let (normal, view, f0) = (Vec3::Y, view_at(30.0), Vec3::splat(0.9));
    let front = ltc::rectangle_specular(&shape, Vec3::ZERO, normal, view, f0, 0.4).unwrap();
    assert!(front.min_element() > 0.0, "{front}");
    let turned = panel(shape.center, Vec3::Y, Vec2::new(1.0, 1.0));
    let back = ltc::rectangle_specular(&turned, Vec3::ZERO, normal, view, f0, 0.4);
    assert_eq!(back, Some(Vec3::ZERO));
    // Entirely below the surface's horizon.
    let below = panel(Vec3::new(0.0, -2.0, 0.0), Vec3::Y, Vec2::new(1.0, 1.0));
    let below = ltc::rectangle_specular(&below, Vec3::ZERO, normal, view, f0, 0.4);
    assert_eq!(below, Some(Vec3::ZERO));
}

#[test]
fn tubes_and_tiny_rectangles_use_the_representative_point() {
    let mut shape = panel(Vec3::new(0.0, 2.0, 0.0), Vec3::NEG_Y, Vec2::new(1.0, 1.0));
    let specular = |shape: &AreaShape| {
        ltc::rectangle_specular(shape, Vec3::ZERO, Vec3::Y, Vec3::Y, Vec3::ONE, 0.5)
    };
    assert!(
        specular(&shape).is_some_and(|light| light.is_finite()),
        "head-on view"
    );
    shape.half = Vec2::splat(1.0e-4);
    assert_eq!(specular(&shape), None);
    shape.half = Vec2::new(0.5, 0.0);
    shape.rectangle = false;
    assert_eq!(specular(&shape), None);
}

#[test]
fn small_rectangles_reflect_like_points() {
    for (roughness, angle) in [(0.3, 20.0), (0.6, 50.0)] {
        let view = view_at(angle);
        let center = Vec3::new(-1.2, 2.0, 0.3);
        let shape = panel(center, -center, Vec2::splat(0.02));
        let light =
            ltc::rectangle_specular(&shape, Vec3::ZERO, Vec3::Y, view, Vec3::ONE, roughness);
        let point = PI * ggx_cosine(Vec3::Y, view, center.normalize(), roughness);
        let light = light.unwrap().x;
        assert!(
            (light - point).abs() < 0.1 * point,
            "r {roughness}: {light} vs {point}"
        );
    }
}

#[test]
fn larger_rectangles_spread_their_highlight() {
    let view = view_at(40.0);
    // The mirror direction meets the panel's plane at x = −2 tan 40°.
    let center = Vec3::new(-2.0 * 40.0f32.to_radians().tan(), 2.0, 0.0);
    let at = |offset: f32, size: f32| {
        let shape = panel(
            center + Vec3::new(offset, 0.0, 0.0),
            Vec3::NEG_Y,
            Vec2::splat(size),
        );
        ltc::rectangle_specular(&shape, Vec3::ZERO, Vec3::Y, view, Vec3::ONE, 0.3)
            .unwrap()
            .x
    };
    assert!(at(0.0, 2.0) < at(0.0, 0.2), "the peak flattens");
    // A metre off the mirror direction, relative to the peak.
    let reach = |size: f32| at(-1.0, size) / at(0.0, size);
    assert!(
        reach(2.0) > 3.0 * reach(0.2),
        "{} vs {}",
        reach(2.0),
        reach(0.2)
    );
}
