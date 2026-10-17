// Light BVH traversal for meshes without bake data (Henry's `DYNAMIC_LIGHTING_BVH` path): only
// lights whose leaf bounds contain the fragment are visited. Nodes live in the aux buffer, see
// `bake/light_bvh.rs` for the 32-byte layout.

import dynlight::baked_lighting::shaders::baked_data::{baked_aux, AUX_BVH, AUX_BVH_NODE_COUNT};

const BVH_NODE_WORDS: u32 = 8u;
const BVH_STACK_SIZE: u32 = 32u;

struct BvhNode {
    aabb_min: vec3<f32>,
    // Leaf: first light slot. Inner node: left child; the right child follows it.
    left_first: u32,
    aabb_max: vec3<f32>,
    // Lights in a leaf; 0 for inner nodes.
    count: u32,
}

// The node stack of the walk in progress. It lives outside `BvhWalk` because Direct3D's FXC
// compiler can't write to an array inside a struct at a run-time index; one walk runs at a time.
var<private> bvh_stack: array<u32, BVH_STACK_SIZE>;

// A traversal in progress; advance it with `bvh_next`. Its nodes are on `bvh_stack`.
struct BvhWalk {
    pointer: u32,
    // Remaining lights of the current leaf.
    next_light: u32,
    end_light: u32,
    // The light slot `bvh_next` found.
    light: u32,
    // Node visits left; bounds the walk even on corrupt data.
    budget: u32,
}

fn bvh_node_count() -> u32 {
    return baked_aux[AUX_BVH_NODE_COUNT];
}

fn bvh_node(index: u32) -> BvhNode {
    let o = baked_aux[AUX_BVH] + index * BVH_NODE_WORDS;
    return BvhNode(
        vec3(bitcast<f32>(baked_aux[o]), bitcast<f32>(baked_aux[o + 1u]), bitcast<f32>(baked_aux[o + 2u])),
        baked_aux[o + 3u],
        vec3(bitcast<f32>(baked_aux[o + 4u]), bitcast<f32>(baked_aux[o + 5u]), bitcast<f32>(baked_aux[o + 6u])),
        baked_aux[o + 7u],
    );
}

fn contains(node: BvhNode, point: vec3<f32>) -> bool {
    return all(point >= node.aabb_min) && all(point <= node.aabb_max);
}

// Starts at the root; a scene without baked lights has no nodes and yields nothing.
fn bvh_start() -> BvhWalk {
    var walk: BvhWalk;
    walk.pointer = 0u;
    walk.next_light = 0u;
    walk.end_light = 0u;
    walk.light = 0u;
    let nodes = bvh_node_count();
    walk.budget = nodes;
    if nodes > 0u {
        bvh_stack[0] = 0u;
        walk.pointer = 1u;
    }
    return walk;
}

// Moves to the next light whose leaf contains `point` and stores its slot in `light`. Returns
// false once the walk is done.
fn bvh_next(walk: ptr<function, BvhWalk>, point: vec3<f32>) -> bool {
    loop {
        if (*walk).next_light < (*walk).end_light {
            (*walk).light = (*walk).next_light;
            (*walk).next_light += 1u;
            return true;
        }
        if (*walk).pointer == 0u || (*walk).budget == 0u {
            return false;
        }
        (*walk).budget -= 1u;
        (*walk).pointer -= 1u;
        let node = bvh_node(bvh_stack[(*walk).pointer]);
        if node.count > 0u {
            (*walk).next_light = node.left_first;
            (*walk).end_light = node.left_first + node.count;
            continue;
        }
        for (var child = 0u; child < 2u; child++) {
            let index = node.left_first + child;
            if (*walk).pointer < BVH_STACK_SIZE && contains(bvh_node(index), point) {
                bvh_stack[(*walk).pointer] = index;
                (*walk).pointer += 1u;
            }
        }
    }
    return false;
}
