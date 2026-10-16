//! The light bounding volume hierarchy: Henry's `BvhLightStructure` (SAH splits at light
//! positions, leaves of at most two lights). Building it also fixes the baked light order: slot
//! `i` of the GPU light buffer holds the light at BVH position `i`.
//!
//! The shadow points of one area light count as one light here, split at their mean position:
//! they share a leaf and take consecutive slots in their input order, so the shaders can shade
//! the area light once and add up its points' shadows. Without area lights the hierarchy is
//! Henry's.

use core::ops::Range;

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

/// The hierarchy is built over items: single lights, or the shadow points of one area light.
struct Builder {
    /// Where each item splits: the mean position of its lights.
    centers: Vec<Vec3>,
    /// The union of each item's light spheres.
    bounds: Vec<Aabb>,
    indices: Vec<usize>,
    nodes: Vec<LightBvhNode>,
}

/// Builds the hierarchy over light spheres given as (position, radius), in input order. Lights
/// next to each other with the same `groups` value (the shadow points of one area light) stay
/// together. No lights give no nodes; the runtime must skip BVH traversal then.
pub fn build_light_bvh(positions: &[Vec3], radii: &[f32], groups: &[u32]) -> LightBvh {
    debug_assert_eq!(positions.len(), radii.len());
    debug_assert_eq!(positions.len(), groups.len());
    if positions.is_empty() {
        return LightBvh::default();
    }
    let sphere = |light: usize| Aabb::from_sphere(positions[light], radii[light]);
    let items = runs(groups);
    let mut builder = Builder {
        centers: items
            .iter()
            .map(|item| positions[item.clone()].iter().sum::<Vec3>() / item.len() as f32)
            .collect(),
        bounds: items
            .iter()
            .map(|item| {
                item.clone()
                    .fold(Aabb::EMPTY, |aabb, l| aabb.union(sphere(l)))
            })
            .collect(),
        indices: (0..items.len()).collect(),
        nodes: Vec::with_capacity(items.len() * 2),
    };
    builder.nodes.push(LightBvhNode {
        left_first: 0,
        count: items.len() as u32,
        ..Default::default()
    });
    builder.update_bounds(0);
    builder.subdivide(0);

    // Leaves cover runs of items; turn them into runs of slots.
    let mut first_slot = vec![0usize];
    for &item in &builder.indices {
        first_slot.push(first_slot.last().copied().unwrap_or(0) + items[item].len());
    }
    for node in builder.nodes.iter_mut().filter(|node| node.is_leaf()) {
        let (first, end) = (
            node.left_first as usize,
            (node.left_first + node.count) as usize,
        );
        node.left_first = first_slot[first] as u32;
        node.count = (first_slot[end] - first_slot[first]) as u32;
    }
    let order: Vec<usize> = builder
        .indices
        .iter()
        .flat_map(|&item| items[item].clone())
        .collect();
    let light_bounds = order.iter().map(|&light| sphere(light)).collect();
    LightBvh {
        nodes: builder.nodes,
        order,
        light_bounds,
    }
}

/// The runs of equal values in `groups`.
fn runs(groups: &[u32]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for (light, group) in groups.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if groups[run.start] == *group => run.end = light + 1,
            _ => runs.push(light..light + 1),
        }
    }
    runs
}

impl Builder {
    fn bounds_of(&self, item: usize) -> Aabb {
        self.bounds[item]
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
                let candidate = self.centers[self.indices[first + k]][axis];
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
            if self.centers[self.indices[i as usize]][axis] < split {
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
        for &item in &self.indices[first..first + count] {
            if self.centers[item][axis] < split {
                left_count += 1;
                left = left.union(self.bounds_of(item));
            } else {
                right_count += 1;
                right = right.union(self.bounds_of(item));
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
