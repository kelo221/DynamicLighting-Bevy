//! The light bounding volume hierarchy: Henry's `BvhLightStructure` (SAH splits at light
//! positions, leaves of at most two lights). Building it also fixes the baked light order: slot
//! `i` of the GPU light buffer holds the light at BVH position `i`.

use bevy::math::Vec3;
use bytemuck::{Pod, Zeroable};

use super::geometry::Aabb;

/// One BVH node as the shaders read it, 32 bytes.
///
/// A leaf has `count > 0` and covers lights `left_first .. left_first + count` (light buffer
/// slots). An inner node has `count == 0`; its children are nodes `left_first` and
/// `left_first + 1`. Node 0 is the root.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct LightBvhNode {
    pub aabb_min: [f32; 3],
    pub left_first: u32,
    pub aabb_max: [f32; 3],
    pub count: u32,
}

/// Size in bytes of one [`LightBvhNode`].
pub const LIGHT_BVH_NODE_SIZE: usize = 32;

impl LightBvhNode {
    pub fn is_leaf(&self) -> bool {
        self.count > 0
    }

    pub fn aabb(&self) -> Aabb {
        Aabb {
            min: Vec3::from(self.aabb_min),
            max: Vec3::from(self.aabb_max),
        }
    }
}

/// A built hierarchy plus the light order it implies.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightBvh {
    pub nodes: Vec<LightBvhNode>,
    /// `order[slot]` is the input index of the light in BVH slot `slot`.
    pub order: Vec<usize>,
    /// Bounds of each light's sphere, in BVH order.
    pub light_bounds: Vec<Aabb>,
}

struct Builder<'a> {
    positions: &'a [Vec3],
    radii: &'a [f32],
    indices: Vec<usize>,
    nodes: Vec<LightBvhNode>,
}

/// Builds the hierarchy over light spheres given as (position, radius), in input order.
/// No lights give no nodes; the runtime must skip BVH traversal then.
pub fn build_light_bvh(positions: &[Vec3], radii: &[f32]) -> LightBvh {
    debug_assert_eq!(positions.len(), radii.len());
    if positions.is_empty() {
        return LightBvh::default();
    }
    let mut builder = Builder {
        positions,
        radii,
        indices: (0..positions.len()).collect(),
        nodes: Vec::with_capacity(positions.len() * 2),
    };
    builder.nodes.push(LightBvhNode {
        left_first: 0,
        count: positions.len() as u32,
        ..Default::default()
    });
    builder.update_bounds(0);
    builder.subdivide(0);
    let light_bounds = builder
        .indices
        .iter()
        .map(|&i| Aabb::from_sphere(positions[i], radii[i]))
        .collect();
    LightBvh {
        nodes: builder.nodes,
        order: builder.indices,
        light_bounds,
    }
}

impl Builder<'_> {
    fn bounds_of(&self, light: usize) -> Aabb {
        Aabb::from_sphere(self.positions[light], self.radii[light])
    }

    fn update_bounds(&mut self, node: usize) {
        let LightBvhNode {
            left_first, count, ..
        } = self.nodes[node];
        let first = left_first as usize;
        let aabb = self.indices[first..first + count as usize]
            .iter()
            .fold(Aabb::EMPTY, |aabb, &light| {
                aabb.union(self.bounds_of(light))
            });
        self.nodes[node].aabb_min = aabb.min.into();
        self.nodes[node].aabb_max = aabb.max.into();
    }

    fn subdivide(&mut self, node: usize) {
        let LightBvhNode {
            left_first, count, ..
        } = self.nodes[node];
        if count <= 2 {
            return;
        }
        let (first, count) = (left_first as usize, count as usize);
        let parent_cost = count as f32 * self.nodes[node].aabb().half_area_x2();

        let mut best: Option<(usize, f32)> = None;
        let mut best_cost = f32::MAX;
        for axis in 0..3 {
            for k in 0..count {
                let candidate = self.positions[self.indices[first + k]][axis];
                let cost = self.split_cost(first, count, axis, candidate);
                if cost < best_cost {
                    best_cost = cost;
                    best = Some((axis, candidate));
                }
            }
        }
        let Some((axis, split)) = best else {
            return;
        };
        if best_cost >= parent_cost {
            return;
        }

        // Henry's in-place partition: lights below the split go left.
        let mut i = first as isize;
        let mut j = (first + count) as isize - 1;
        while i <= j {
            if self.positions[self.indices[i as usize]][axis] < split {
                i += 1;
            } else {
                self.indices.swap(i as usize, j as usize);
                j -= 1;
            }
        }
        let left_count = i as usize - first;
        if left_count == 0 || left_count == count {
            return;
        }

        let left = self.nodes.len();
        self.nodes.push(LightBvhNode {
            left_first: first as u32,
            count: left_count as u32,
            ..Default::default()
        });
        self.nodes.push(LightBvhNode {
            left_first: i as u32,
            count: (count - left_count) as u32,
            ..Default::default()
        });
        self.nodes[node].left_first = left as u32;
        self.nodes[node].count = 0;
        self.update_bounds(left);
        self.update_bounds(left + 1);
        self.subdivide(left);
        self.subdivide(left + 1);
    }

    fn split_cost(&self, first: usize, count: usize, axis: usize, split: f32) -> f32 {
        let (mut left, mut right) = (Aabb::EMPTY, Aabb::EMPTY);
        let (mut left_count, mut right_count) = (0u32, 0u32);
        for &light in &self.indices[first..first + count] {
            if self.positions[light][axis] < split {
                left_count += 1;
                left = left.union(self.bounds_of(light));
            } else {
                right_count += 1;
                right = right.union(self.bounds_of(light));
            }
        }
        if left_count == 0 || right_count == 0 {
            return f32::MAX;
        }
        left_count as f32 * left.half_area_x2() + right_count as f32 * right.half_area_x2()
    }
}

impl LightBvh {
    /// Appends the BVH slots of every light whose sphere bounds overlap `aabb`, ascending.
    ///
    /// Henry's query returns whole leaves; this also tests each light's own bounds, so the
    /// result equals a brute-force scan.
    pub fn query(&self, aabb: &Aabb, out: &mut Vec<u32>) {
        let start = out.len();
        let Some(root) = self.nodes.first() else {
            return;
        };
        let mut stack = vec![*root];
        while let Some(node) = stack.pop() {
            if node.is_leaf() {
                for slot in node.left_first..node.left_first + node.count {
                    if self.light_bounds[slot as usize].intersects(aabb) {
                        out.push(slot);
                    }
                }
                continue;
            }
            for child in [node.left_first, node.left_first + 1] {
                let child = self.nodes[child as usize];
                if child.aabb().intersects(aabb) {
                    stack.push(child);
                }
            }
        }
        out[start..].sort_unstable();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

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
        let bvh = build_light_bvh(&positions, &radii);
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
        let bvh = build_light_bvh(&positions, &radii);
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
        assert!(build_light_bvh(&[], &[]).nodes.is_empty());
        let one = build_light_bvh(&[Vec3::ONE], &[2.0]);
        assert_eq!(one.nodes.len(), 1);
        assert_eq!((one.nodes[0].left_first, one.nodes[0].count), (0, 1));
        assert_eq!(one.nodes[0].aabb_min, [-1.0; 3]);
        let mut found = Vec::new();
        one.query(&Aabb::from_sphere(Vec3::splat(10.0), 1.0), &mut found);
        assert!(found.is_empty());
        // Lights at the same spot can't be split; they share one leaf.
        let same = build_light_bvh(&[Vec3::ZERO; 5], &[1.0; 5]);
        assert_eq!(same.nodes.len(), 1);
        assert_eq!(same.nodes[0].count, 5);
    }

    #[test]
    fn build_is_deterministic() {
        let (positions, radii) = lights(100);
        assert_eq!(
            build_light_bvh(&positions, &radii),
            build_light_bvh(&positions, &radii)
        );
    }
}
