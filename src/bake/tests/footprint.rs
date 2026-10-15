//! Plane + box occluder + point light: the baked shadow bits on the floor match the analytic
//! shadow footprint, read back through the shader mirror.

use bevy::math::{Vec2, Vec3};

use super::{cuboid, floor, point_light};
use crate::bake::input::{BakeInput, BakeOptions};
use crate::bake::mirror::TriangleView;
use crate::bake::progress::BakeProgress;
use crate::bake::texels::triangle_texels;
use crate::bake::{BakedScene, bake};
use crate::shared::{BakeId, DynamicGeometryMode};

const FLOOR_HALF: f32 = 2.0;
const BOX_MIN: Vec3 = Vec3::new(-0.5, 0.5, -0.5);
const BOX_MAX: Vec3 = Vec3::new(0.5, 1.0, 0.5);

fn options() -> BakeOptions {
    BakeOptions {
        pixel_density: 16.0,
        dynamic_geometry: DynamicGeometryMode::LightingOnly,
        ..Default::default()
    }
}

fn scene(light: Vec3) -> BakeInput {
    BakeInput {
        meshes: vec![
            floor(1, Vec3::ZERO, FLOOR_HALF),
            cuboid(2, BOX_MIN, BOX_MAX),
        ],
        lights: vec![point_light(10, light, 10.0)],
    }
}

/// Whether the segment from `point` to `light` passes through the box (slab test).
fn analytic_shadow(point: Vec3, light: Vec3) -> bool {
    let direction = light - point;
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for axis in 0..3 {
        let (o, d) = (point[axis], direction[axis]);
        let (lo, hi) = (BOX_MIN[axis], BOX_MAX[axis]);
        if d.abs() < 1e-9 {
            if o < lo || o > hi {
                return false;
            }
            continue;
        }
        let (a, b) = ((lo - o) / d, (hi - o) / d);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    t0 <= t1
}

/// Checks every floor texel: (matching, ambiguous, lit, shadowed) counts; panics on a mismatch
/// away from the shadow edge.
fn check_floor(baked: &BakedScene, light: Vec3) -> (u32, u32, u32) {
    let mesh = baked.meshes.iter().find(|m| m.id == BakeId(1)).unwrap();
    let size = mesh.resolution;
    let texel = 2.0 * FLOOR_HALF / size as f32;
    let floor_mesh = floor(1, Vec3::ZERO, FLOOR_HALF);
    let (mut ambiguous, mut lit, mut shadowed) = (0, 0, 0);
    for triangle in 0..2 {
        let corners = &floor_mesh.indices[triangle * 3..][..3];
        let vertices = [0, 1, 2].map(|k| floor_mesh.positions[corners[k] as usize]);
        let uvs: [Vec2; 3] = [0, 1, 2].map(|k| floor_mesh.uv1[corners[k] as usize]);
        let view = TriangleView::load(&mesh.triangles, triangle as u32, baked.settings.bounce);
        let entry = (0..view.light_count)
            .map(|k| view.light(k))
            .find(|e| e.light_index == 0);
        for t in triangle_texels(vertices, [Vec3::Y; 3], uvs, Vec3::Y, size) {
            let expected = !analytic_shadow(t.position, light);
            let neighbours_agree = [-1.0, 0.0, 1.0].iter().all(|&dx| {
                [-1.0, 0.0, 1.0].iter().all(|&dz| {
                    let p = t.position + Vec3::new(dx, 0.0, dz) * texel;
                    !analytic_shadow(p, light) == expected
                })
            });
            if !neighbours_agree {
                ambiguous += 1;
                continue;
            }
            let actual = entry.is_some_and(|e| view.shadow_bit(&e, t.x, t.y) == Some(true));
            assert_eq!(
                actual, expected,
                "texel ({}, {}) at {} with light at {light}",
                t.x, t.y, t.position
            );
            if expected {
                lit += 1;
            } else {
                shadowed += 1;
            }
        }
    }
    (ambiguous, lit, shadowed)
}

#[test]
fn shadow_under_a_box_matches_the_projection() {
    let light = Vec3::new(0.0, 3.0, 0.0);
    let baked = bake(&scene(light), &options(), &BakeProgress::new()).unwrap();
    assert_eq!(baked.meshes[0].resolution, 64);
    let (ambiguous, lit, shadowed) = check_floor(&baked, light);
    // The shadow is the box's top face projected from the light: a 1.5 m square, 24² texels.
    let side = 1.5 * 16.0;
    assert!(shadowed as f32 > (side - 2.0) * (side - 2.0), "{shadowed}");
    assert!(shadowed as f32 <= side * side);
    assert!(lit > 3000, "{lit}");
    assert!(ambiguous < 4 * 26 * 3, "{ambiguous}");
}

#[test]
fn off_center_light_casts_a_skewed_shadow() {
    let light = Vec3::new(1.2, 2.5, -0.7);
    let baked = bake(&scene(light), &options(), &BakeProgress::new()).unwrap();
    let (_, lit, shadowed) = check_floor(&baked, light);
    assert!(shadowed > 300 && lit > 3000, "{shadowed} {lit}");
}

#[test]
fn texels_beyond_the_radius_get_no_light() {
    let light = Vec3::new(0.0, 0.5, 0.0);
    let mut input = scene(light);
    input.meshes.truncate(1);
    input.lights[0].radius = 1.0;
    let baked = bake(&input, &options(), &BakeProgress::new()).unwrap();
    let mesh = &baked.meshes[0];
    for triangle in 0..2 {
        let view = TriangleView::load(&mesh.triangles, triangle, false);
        let entry = view.light(0);
        // Texel 56 is at x = z = 1.53 m, outside the 1 m radius.
        assert_eq!(view.shadow_bit(&entry, 56, 56), Some(false));
        // Right under the light is lit.
        assert_eq!(view.shadow_bit(&entry, 31, 31), Some(true));
    }
}

#[test]
fn walls_facing_away_lose_the_light() {
    // A floor below a light faces it; one above faces away (its normal points up, away from a
    // light below) and is dropped by the facing test during association.
    let mut input = scene(Vec3::new(0.0, 3.0, 0.0));
    input.meshes = vec![floor(1, Vec3::ZERO, 1.0), floor(3, Vec3::Y * 5.0, 1.0)];
    let baked = bake(&input, &options(), &BakeProgress::new()).unwrap();
    let below = &baked.meshes[0];
    let above = &baked.meshes[1];
    assert_eq!(
        TriangleView::load(&below.triangles, 0, false).light_count,
        1
    );
    assert_eq!(
        TriangleView::load(&above.triangles, 0, false).light_count,
        0
    );
    assert_eq!(baked.stats.lights_removed, 0);
}
