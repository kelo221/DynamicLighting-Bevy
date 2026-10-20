//! Suns: parallel shadows matching the analytic projection, penumbras that widen with the sun's
//! disk and the blocker's height, and suns kept out of the light BVH and the distance cubes.
//! Sun bounce is in [`super::colored_bounce`].

use bevy::math::{Vec2, Vec3};

use super::{cuboid, floor, point_light, sun_light};
use crate::bake::input::{BakeInput, BakeOptions};
use crate::bake::mirror::TriangleView;
use crate::bake::progress::BakeProgress;
use crate::bake::texels::triangle_texels;
use crate::bake::{BakedScene, bake};
use crate::shared::{BakeId, DynamicGeometryMode};

const FLOOR_HALF: f32 = 2.0;
const DENSITY: f32 = 16.0;

fn options() -> BakeOptions {
    BakeOptions {
        pixel_density: DENSITY,
        dynamic_geometry: DynamicGeometryMode::LightingOnly,
        sun_map_resolution: 64,
        ..Default::default()
    }
}

/// A floor with a box (`min`, `max`) over it, lit by one sun.
fn bake_sun(toward: Vec3, degrees: f32, min: Vec3, max: Vec3) -> BakedScene {
    let input = BakeInput {
        meshes: vec![floor(1, Vec3::ZERO, FLOOR_HALF), cuboid(2, min, max)],
        lights: vec![sun_light(10, toward, degrees)],
    };
    bake(&input, &options(), &BakeProgress::new()).unwrap()
}

/// Whether the ray from `point` along `direction` passes through the box (slab test).
fn ray_hits_box(point: Vec3, direction: Vec3, min: Vec3, max: Vec3) -> bool {
    let (mut t0, mut t1) = (0.0f32, f32::INFINITY);
    for axis in 0..3 {
        let (o, d) = (point[axis], direction[axis]);
        if d.abs() < 1e-9 {
            if o < min[axis] || o > max[axis] {
                return false;
            }
            continue;
        }
        let (a, b) = ((min[axis] - o) / d, (max[axis] - o) / d);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    t0 <= t1
}

/// Every floor texel with its baked visibility from light 0, `None` without an entry.
fn floor_texels(baked: &BakedScene) -> Vec<(Vec3, Option<f32>)> {
    let mesh = baked.meshes.iter().find(|m| m.id == BakeId(1)).unwrap();
    let floor_mesh = floor(1, Vec3::ZERO, FLOOR_HALF);
    let bits = baked.lights[0].shadow_bits_per_texel;
    let mut texels = Vec::new();
    for triangle in 0..2 {
        let corners = &floor_mesh.indices[triangle * 3..][..3];
        let vertices = [0, 1, 2].map(|k| floor_mesh.positions[corners[k] as usize]);
        let uvs: [Vec2; 3] = [0, 1, 2].map(|k| floor_mesh.uv1[corners[k] as usize]);
        let view = TriangleView::load(&mesh.triangles, triangle as u32, baked.settings.bounce);
        let entry = (0..view.light_count)
            .map(|k| view.light(k))
            .find(|e| e.light_index == 0);
        for t in triangle_texels(vertices, [Vec3::Y; 3], uvs, Vec3::Y, mesh.resolution) {
            let value = entry.and_then(|e| view.shadow_value(&e, t.x, t.y, bits));
            texels.push((t.position, value));
        }
    }
    texels
}

#[test]
fn hard_sun_shadows_are_the_parallel_projection() {
    let (min, max) = (Vec3::new(-0.5, 0.5, -0.5), Vec3::new(0.5, 1.0, 0.5));
    let toward = Vec3::new(0.4, 1.0, -0.3).normalize();
    let baked = bake_sun(toward, 0.0, min, max);
    assert_eq!(baked.lights[0].shadow_bits_per_texel, 1);
    let texel = 2.0 * FLOOR_HALF / baked.meshes[0].resolution as f32;
    let (mut lit, mut shadowed) = (0, 0);
    for (position, value) in floor_texels(&baked) {
        let expected = !ray_hits_box(position, toward, min, max);
        let edge = [-1.0, 0.0, 1.0].iter().any(|&dx| {
            [-1.0, 0.0, 1.0].iter().any(|&dz| {
                let p = position + Vec3::new(dx, 0.0, dz) * texel;
                ray_hits_box(p, toward, min, max) == expected
            })
        });
        if edge {
            continue;
        }
        let value = value.expect("every floor texel sees the sun");
        assert_eq!(value == 1.0, expected, "{position}");
        if expected {
            lit += 1;
        } else {
            shadowed += 1;
        }
    }
    // The box projected along the sun: its top and bottom squares 0.2 m and 0.15 m apart, so
    // 1.35 m² or 345 texels, less the edge texels skipped above.
    assert!(shadowed > 200 && shadowed < 346, "{shadowed}");
    assert!(lit > 3000, "{lit}");
}

/// Floor texels strictly between fully lit and fully shadowed.
fn penumbra_texels(degrees: f32, height: f32) -> usize {
    let min = Vec3::new(-0.5, height, -0.5);
    let baked = bake_sun(Vec3::Y, degrees, min, min + Vec3::new(1.0, 0.2, 1.0));
    floor_texels(&baked)
        .into_iter()
        .filter(|(_, value)| value.is_some_and(|v| v > 0.0 && v < 1.0))
        .count()
}

#[test]
fn penumbras_widen_with_the_disk_and_the_blocker_height() {
    assert_eq!(penumbra_texels(0.0, 1.0), 0, "a sun without a disk is hard");
    let narrow = penumbra_texels(4.0, 1.0);
    let wide = penumbra_texels(10.0, 1.0);
    let high = penumbra_texels(10.0, 1.6);
    assert!(
        narrow > 0 && narrow < wide && wide < high,
        "{narrow} {wide} {high}"
    );
    // A 10° disk 1 m up spreads each edge over 2 · tan 5° = 0.17 m, under 3 texels: the four
    // edges of the 1 m square hold about 4 · 16 · 3 texels.
    assert!(wide < 4 * 18 * 4, "{wide}");
}

#[test]
fn suns_skip_the_light_bvh_and_distance_cubes() {
    let sun = sun_light(5, Vec3::new(0.2, 1.0, 0.1), 0.53);
    let input = BakeInput {
        meshes: vec![
            floor(1, Vec3::ZERO, FLOOR_HALF),
            // A small roof facing up, which faces away from the point light below it.
            floor(3, Vec3::Y * 3.0, 0.5),
        ],
        lights: vec![sun, point_light(9, Vec3::new(0.0, 1.0, 0.0), 2.0)],
    };
    let options = BakeOptions {
        dynamic_geometry: DynamicGeometryMode::Angular,
        ..options()
    };
    let baked = bake(&input, &options, &BakeProgress::new()).unwrap();
    let ids: Vec<u64> = baked.lights.iter().map(|light| light.id.0).collect();
    assert_eq!(ids, [9, 5], "suns come last whatever their id");
    let root = baked.light_bvh[0];
    assert_eq!(root.count, 1, "the BVH holds the point light only");
    assert_eq!(baked.distance_cubes.len(), 12_288);
    assert_eq!(baked.sun_maps.len(), 1);
    assert_eq!(baked.lights[1].bounce_compression, None);
    assert!(!baked.settings.bounce);
    // The sun reaches every triangle facing it, wherever it is.
    let roof = baked.meshes.iter().find(|m| m.id == BakeId(3)).unwrap();
    let view = TriangleView::load(&roof.triangles, 0, false);
    let lights: Vec<u32> = (0..view.light_count)
        .map(|k| view.light(k).light_index)
        .collect();
    assert_eq!(lights, [1]);
}

#[test]
fn lighting_only_bakes_no_sun_maps() {
    let baked = bake_sun(Vec3::Y, 0.53, Vec3::ONE, Vec3::ONE * 1.5);
    assert!(baked.sun_maps.is_empty() && baked.distance_cubes.is_empty());
}
