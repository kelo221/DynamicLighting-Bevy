//! The light BVH: Henry's leaves, queries against brute force, and area lights kept together.

use std::mem::offset_of;

use bevy::math::Vec3;

use super::super::geometry::Aabb;
use super::super::light_bvh::{LIGHT_BVH_NODE_SIZE, LightBvhNode, build_light_bvh};

/// Every light its own group.
fn single(count: usize) -> Vec<u32> {
    (0..count as u32).collect()
}

fn lights(count: usize) -> (Vec<Vec3>, Vec<f32>) {
    let mut state = 0x1234_5678_u64;
    let mut next = move || {
        state = crate::effects::splitmix64(state);
        (state >> 40) as f32 / (1u64 << 24) as f32
    };
    (0..count)
        .map(|_| {
            let p = Vec3::new(next(), next(), next()) * 40.0 - 20.0;
            (p, 0.5 + next() * 4.0)
        })
        .unzip()
}

#[test]
fn node_layout_matches_the_shader_struct() {
    assert_eq!(size_of::<LightBvhNode>(), LIGHT_BVH_NODE_SIZE);
    assert_eq!(offset_of!(LightBvhNode, aabb_min), 0);
    assert_eq!(offset_of!(LightBvhNode, left_first), 12);
    assert_eq!(offset_of!(LightBvhNode, aabb_max), 16);
    assert_eq!(offset_of!(LightBvhNode, count), 28);
}

#[test]
fn leaves_partition_the_lights_and_hold_at_most_two() {
    let (positions, radii) = lights(57);
    let bvh = build_light_bvh(&positions, &radii, &single(positions.len()));
    let mut order = bvh.order.clone();
    order.sort_unstable();
    assert_eq!(order, (0..57).collect::<Vec<_>>());
    let mut covered = vec![0u32; 57];
    for node in bvh.nodes.iter().filter(|n| n.is_leaf()) {
        for slot in node.left_first..node.left_first + node.count {
            covered[slot as usize] += 1;
            // Leaves contain their lights.
            let light = bvh.light_bounds[slot as usize];
            assert!(node.aabb().min.cmple(light.min).all());
            assert!(node.aabb().max.cmpge(light.max).all());
        }
    }
    assert!(covered.iter().all(|&c| c == 1));
    assert!(bvh.nodes.len() < 2 * 57);
    assert!(
        bvh.nodes
            .iter()
            .filter(|n| n.is_leaf())
            .all(|n| n.count <= 2)
    );
}

#[test]
fn query_equals_brute_force() {
    let (positions, radii) = lights(200);
    let bvh = build_light_bvh(&positions, &radii, &single(positions.len()));
    let (centers, sizes) = lights(300);
    for (center, size) in centers.iter().zip(sizes) {
        let query = Aabb::from_sphere(*center, size * 0.5);
        let mut found = Vec::new();
        bvh.query(&query, &mut found);
        let expected: Vec<u32> = (0..200u32)
            .filter(|&slot| bvh.light_bounds[slot as usize].intersects(&query))
            .collect();
        assert_eq!(found, expected);
    }
}

#[test]
fn small_and_empty_sets() {
    assert!(build_light_bvh(&[], &[], &[]).nodes.is_empty());
    let one = build_light_bvh(&[Vec3::ONE], &[2.0], &[0]);
    assert_eq!(one.nodes.len(), 1);
    assert_eq!((one.nodes[0].left_first, one.nodes[0].count), (0, 1));
    assert_eq!(one.nodes[0].aabb_min, [-1.0; 3]);
    let mut found = Vec::new();
    one.query(&Aabb::from_sphere(Vec3::splat(10.0), 1.0), &mut found);
    assert!(found.is_empty());
    // Lights at the same spot can't be split; they share one leaf.
    let same = build_light_bvh(&[Vec3::ZERO; 5], &[1.0; 5], &single(5));
    assert_eq!(same.nodes.len(), 1);
    assert_eq!(same.nodes[0].count, 5);
}

#[test]
fn build_is_deterministic() {
    let (positions, radii) = lights(100);
    assert_eq!(
        build_light_bvh(&positions, &radii, &single(positions.len())),
        build_light_bvh(&positions, &radii, &single(positions.len()))
    );
}

/// 40 lights; lights 10 to 15 and 30 to 33 are the shadow points of two area lights.
fn with_area_lights() -> (Vec<Vec3>, Vec<f32>, Vec<u32>) {
    let (positions, radii) = lights(40);
    let mut groups = Vec::new();
    let mut group = 0;
    for light in 0..40 {
        if !(11..=15).contains(&light) && !(31..=33).contains(&light) {
            group += 1;
        }
        groups.push(group);
    }
    (positions, radii, groups)
}

#[test]
fn area_lights_take_consecutive_slots_in_one_leaf() {
    let (positions, radii, groups) = with_area_lights();
    let bvh = build_light_bvh(&positions, &radii, &groups);
    let mut sorted = bvh.order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..40).collect::<Vec<_>>());
    for points in [10..16, 30..34] {
        let first = bvh.order.iter().position(|&l| l == points.start).unwrap();
        let slots = &bvh.order[first..first + points.len()];
        assert_eq!(slots, points.clone().collect::<Vec<_>>(), "in point order");
        let leaf = bvh
            .nodes
            .iter()
            .find(|n| {
                n.is_leaf() && (n.left_first..n.left_first + n.count).contains(&(first as u32))
            })
            .unwrap();
        assert!(leaf.left_first as usize <= first);
        assert!((leaf.left_first + leaf.count) as usize >= first + points.len());
    }
    let mut covered = [0u32; 40];
    for node in bvh.nodes.iter().filter(|n| n.is_leaf()) {
        for slot in node.left_first..node.left_first + node.count {
            covered[slot as usize] += 1;
        }
    }
    assert!(covered.iter().all(|&c| c == 1));
}

#[test]
fn grouped_queries_equal_brute_force() {
    let (positions, radii, groups) = with_area_lights();
    let bvh = build_light_bvh(&positions, &radii, &groups);
    assert_eq!(bvh, build_light_bvh(&positions, &radii, &groups));
    let (centers, sizes) = lights(300);
    for (center, size) in centers.iter().zip(sizes) {
        let query = Aabb::from_sphere(*center, size * 0.5);
        let mut found = Vec::new();
        bvh.query(&query, &mut found);
        let expected: Vec<u32> = (0..40u32)
            .filter(|&slot| bvh.light_bounds[slot as usize].intersects(&query))
            .collect();
        assert_eq!(found, expected);
    }
}
