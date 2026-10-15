// The baked-data buffers and their header. Mirrors `baked_lighting/layout.rs`.
//
// Any material that wants baked shadows binds these two buffers at 23 and 24 of its bind group
// (`DynLitMaterial` does by default).

@group(constants::MATERIAL_BIND_GROUP) @binding(23) var<storage, read> baked_triangles: array<u32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(24) var<storage, read> baked_aux: array<u32>;

const AUX_RECEIVERS: u32 = 0u;
const AUX_RECEIVER_COUNT: u32 = 1u;
const AUX_BVH: u32 = 2u;
const AUX_BVH_NODE_COUNT: u32 = 3u;
const AUX_CUBES: u32 = 4u;
const AUX_CUBE_COUNT: u32 = 5u;
const AUX_FLAGS: u32 = 6u;
const AUX_BAKED_LIGHTS: u32 = 7u;

const AUX_FLAG_BOUNCE_LAYOUT: u32 = 1u;
const AUX_FLAG_CUBE_BIAS: u32 = 2u;
const AUX_FLAG_CUBE_ANGULAR: u32 = 4u;

const RECEIVER_WORDS: u32 = 4u;

// A baked mesh: where its triangle words start and its shadow map size. `resolution == 0`
// means the mesh has no bake data and takes the dynamic-geometry path.
struct Receiver {
    triangles_base: u32,
    resolution: u32,
    triangle_count: u32,
}

fn aux_flags() -> u32 {
    return baked_aux[AUX_FLAGS];
}

// The receiver selected by a `MeshTag`; tag 0 and unknown tags give an unbaked receiver.
fn receiver(tag: u32) -> Receiver {
    var record = Receiver(0u, 0u, 0u);
    if tag == 0u || tag > baked_aux[AUX_RECEIVER_COUNT] {
        return record;
    }
    let offset = baked_aux[AUX_RECEIVERS] + (tag - 1u) * RECEIVER_WORDS;
    record.triangles_base = baked_aux[offset];
    record.resolution = baked_aux[offset + 1u];
    record.triangle_count = baked_aux[offset + 2u];
    return record;
}
