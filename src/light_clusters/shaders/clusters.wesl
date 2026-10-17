// Which realtime lights can reach a fragment: a grid of cells over the view of the camera that
// chose the realtime lights, each listing the realtime lights whose sphere reaches it. Mirrors
// `light_clusters/grid.rs` and `light_clusters/assign.rs`.
//
// Any material that wants the shorter lists binds the buffer at 29 of its bind group
// (`DynLitMaterial` does by default).

// Mirrors `GpuClusterHeader`. All zero when the grid is off.
struct ClusterHeader {
    clip_from_world: mat4x4<f32>,
    depth_row: vec4<f32>,
    dims: vec3<u32>,
    entry_base: u32,
    first_depth: f32,
    far: f32,
    slice_scale: f32,
    slice_bias: f32,
    flags: u32,
}

const CLUSTER_FLAG_PERSPECTIVE: u32 = 1u;

// Each cell's first entry and entry count, then the entries: realtime light indices, two 16-bit
// indices per word.
struct RealtimeClusters {
    header: ClusterHeader,
    words: array<u32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(29) var<storage, read> realtime_clusters: RealtimeClusters;

// The realtime lights to visit: entries `first..first + count` of a cell, or, when not
// `clustered`, realtime lights `0..count`.
struct RealtimeSpan {
    first: u32,
    count: u32,
    clustered: bool,
}

// The realtime lights that can reach `world`, out of `realtime_count`.
fn realtime_span(world: vec3<f32>, realtime_count: u32) -> RealtimeSpan {
    let every = RealtimeSpan(0u, realtime_count, false);
    let dims = realtime_clusters.header.dims;
    if dims.x == 0u {
        return every;
    }
    let clip = realtime_clusters.header.clip_from_world * vec4(world, 1.0);
    if clip.w <= 0.0 {
        return every;
    }
    let ndc = clip.xy / clip.w;
    if abs(ndc.x) > 1.0 || abs(ndc.y) > 1.0 {
        return every;
    }
    let row = realtime_clusters.header.depth_row;
    let depth = dot(row.xyz, world) + row.w;
    let scale = realtime_clusters.header.slice_scale;
    let bias = realtime_clusters.header.slice_bias;
    var slice = depth * scale + bias;
    if (realtime_clusters.header.flags & CLUSTER_FLAG_PERSPECTIVE) != 0u {
        slice = select(1.0 + log2(depth) * scale + bias, 0.0, depth < realtime_clusters.header.first_depth);
    }
    if depth > realtime_clusters.header.far || slice < 0.0 {
        return RealtimeSpan(0u, 0u, true);
    }
    let tile = vec2<u32>((ndc * 0.5 + 0.5) * vec2<f32>(dims.xy));
    let x = min(tile.x, dims.x - 1u);
    let y = min(tile.y, dims.y - 1u);
    let z = min(u32(slice), dims.z - 1u);
    let cell = (z * dims.y + y) * dims.x + x;
    return RealtimeSpan(realtime_clusters.words[2u * cell], realtime_clusters.words[2u * cell + 1u], true);
}

// The realtime light index (0 is the first realtime light) of the `i`th light of a span.
fn span_light(span: RealtimeSpan, i: u32) -> u32 {
    if !span.clustered {
        return i;
    }
    let entry = span.first + i;
    let word = realtime_clusters.words[realtime_clusters.header.entry_base + (entry >> 1u)];
    return (word >> ((entry & 1u) * 16u)) & 0xffffu;
}
