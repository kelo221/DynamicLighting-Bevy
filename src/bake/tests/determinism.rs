//! Same input, same bytes: across repeated bakes, thread counts and input order.

use bevy::math::{Quat, Vec2, Vec3};

use super::{area_light, cuboid, floor, point_light, sky_light, sun_light};
use crate::bake::input::{BakeError, BakeInput, BakeOptions};
use crate::bake::progress::BakeProgress;
use crate::bake::{bake, encode};
use crate::shared::{BakeId, DynamicGeometryMode};

fn room() -> BakeInput {
    BakeInput {
        meshes: vec![
            floor(5, Vec3::ZERO, 3.0),
            cuboid(3, Vec3::new(-1.0, 0.0, -1.0), Vec3::new(0.0, 1.0, 0.5)),
            cuboid(9, Vec3::new(1.0, 0.2, 1.0), Vec3::new(1.5, 2.0, 1.4)),
            floor(4, Vec3::new(0.5, 2.5, 0.0), 1.0),
        ],
        lights: vec![
            point_light(30, Vec3::new(0.0, 2.0, 0.0), 4.0),
            point_light(10, Vec3::new(-2.0, 1.5, 2.0), 3.0),
            point_light(20, Vec3::new(2.0, 1.0, -2.0), 5.0),
            point_light(40, Vec3::new(0.5, 3.0, 0.2), 2.0),
            point_light(50, Vec3::new(-2.5, 0.3, -2.5), 1.5),
            area_light(
                60,
                Vec3::new(0.2, 2.4, -0.3),
                4.0,
                Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2),
                Vec2::new(1.0, 0.6),
            ),
            area_light(
                70,
                Vec3::new(-1.5, 1.8, 0.8),
                3.0,
                Quat::from_rotation_y(0.7),
                Vec2::new(1.2, 0.0),
            ),
            sun_light(25, Vec3::new(0.3, 1.0, 0.2), 0.53),
            sun_light(15, Vec3::new(-0.5, 1.0, 0.4), 0.0),
            sky_light(12),
        ],
    }
}

fn options() -> BakeOptions {
    BakeOptions {
        pixel_density: 12.0,
        dynamic_geometry: DynamicGeometryMode::Angular,
        sun_map_resolution: 64,
        sky_samples: 16,
        sky_probe_spacing: 0.75,
        ..Default::default()
    }
}

fn bake_bytes(input: &BakeInput, threads: usize) -> Vec<u8> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    pool.install(|| encode(&bake(input, &options(), &BakeProgress::new()).unwrap()))
}

#[test]
fn bakes_are_byte_identical_on_any_thread_count() {
    let input = room();
    let single = bake_bytes(&input, 1);
    assert_eq!(single, bake_bytes(&input, 1), "repeated bake");
    assert_eq!(single, bake_bytes(&input, 4), "4 threads");
    assert_eq!(single, bake_bytes(&input, 7), "7 threads");
}

#[test]
fn input_order_does_not_matter() {
    let input = room();
    let mut shuffled = input.clone();
    shuffled.meshes.reverse();
    shuffled.lights.rotate_left(2);
    assert_eq!(bake_bytes(&input, 2), bake_bytes(&shuffled, 2));
}

#[test]
fn outputs_are_sorted_and_lights_follow_the_bvh() {
    let baked = bake(&room(), &options(), &BakeProgress::new()).unwrap();
    let ids: Vec<u64> = baked.meshes.iter().map(|m| m.id.0).collect();
    assert_eq!(ids, [3, 4, 5, 9]);
    assert_eq!(baked.lights.len(), 10);
    // The suns follow the BVH's lights by id, then the sky, whatever its id.
    let distant: Vec<u64> = baked.lights[7..].iter().map(|light| light.id.0).collect();
    assert_eq!(distant, [15, 25, 12]);
    let counts = (
        baked.local_light_count(),
        baked.distant_count(),
        baked.sun_count(),
    );
    assert_eq!(counts, (7, 3, 2));
    assert_eq!(baked.lights[9].shadow_bits_per_texel, 8);
    assert!(baked.sky_probes.is_some());
    // Every leaf's lights lie inside it: the light list is in BVH order.
    for node in baked.light_bvh.iter().filter(|n| n.is_leaf()) {
        for light in &baked.lights[node.left_first as usize..][..node.count as usize] {
            let min = light.origin - light.radius;
            let max = light.origin + light.radius;
            assert!(Vec3::from(node.aabb_min).cmple(min + 1e-5).all());
            assert!(Vec3::from(node.aabb_max).cmpge(max - 1e-5).all());
        }
    }
    assert_eq!(baked.distance_cubes.len(), 7 * 12_288, "no cubes for suns");
    assert_eq!(baked.sun_maps.len(), 2);
    let soft = baked
        .lights
        .iter()
        .filter(|light| light.shadow_bits_per_texel == 4);
    assert_eq!(
        soft.count(),
        3,
        "the panel, the tube and the wide sun are soft"
    );
    assert!(baked.stats.rays > 0 && baked.stats.texels > 0);
    assert_eq!(baked.stats.meshes, 4);
}

#[test]
fn duplicate_ids_and_bad_lights_are_errors() {
    let mut input = room();
    input.meshes[1].id = BakeId(5);
    assert_eq!(
        bake(&input, &options(), &BakeProgress::new()).unwrap_err(),
        BakeError::DuplicateId {
            kind: "meshes",
            id: BakeId(5)
        }
    );
    let mut input = room();
    input.lights[0].radius = 0.0;
    assert!(matches!(
        bake(&input, &options(), &BakeProgress::new()),
        Err(BakeError::InvalidLight { .. })
    ));
    let mut input = room();
    input.meshes[0].indices.push(99);
    assert!(matches!(
        bake(&input, &options(), &BakeProgress::new()),
        Err(BakeError::InvalidMesh { .. })
    ));
}

#[test]
fn cancelled_bakes_stop() {
    let progress = BakeProgress::new();
    progress.cancel();
    assert_eq!(
        bake(&room(), &options(), &progress).unwrap_err(),
        BakeError::Cancelled
    );
}

#[test]
fn empty_scenes_bake() {
    let baked = bake(&BakeInput::default(), &options(), &BakeProgress::new()).unwrap();
    assert!(baked.meshes.is_empty() && baked.lights.is_empty() && baked.light_bvh.is_empty());
    let lightless = BakeInput {
        meshes: vec![floor(1, Vec3::ZERO, 1.0)],
        lights: vec![],
    };
    let baked = bake(&lightless, &options(), &BakeProgress::new()).unwrap();
    assert_eq!(baked.meshes[0].triangle_count(), 2);
}
