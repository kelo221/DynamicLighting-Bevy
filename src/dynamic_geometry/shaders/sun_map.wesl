// Baked sun maps as shadow maps for meshes without bake data: the sun's counterpart of a
// distance cube. Mirrors `dynamic_geometry::sun_mirror`.
//
// Each baked sun has an orthographic map of the depth of the first static surface along its
// rays, unorm16, two texels per word (texel 2i low, 2i + 1 high), rows of increasing y. Its
// record holds three world-to-map rows (x and y across the map, z along the rays, all 0 to 1),
// the resolution and the first texel word. A texel of 1 means nothing is in the way.

import dynlight::baked_lighting::shaders::baked_data::{
    baked_aux, AUX_SUN_MAPS, AUX_SUN_MAP_COUNT, SUN_MAP_RECORD_WORDS, SUN_MAP_RESOLUTION,
    SUN_MAP_TEXELS,
};

// Receivers less than this many meters, plus this many map texels, behind the stored surface
// still count as lit, so meshes resting on static geometry don't shade themselves. The texel
// term covers the filter's reach of up to two texels across sloped surfaces.
const SUN_MAP_BIAS: f32 = 0.05;
const SUN_MAP_TEXEL_BIAS: f32 = 2.0;

// Whether the `sun`-th baked sun has a sun map.
fn sun_map_available(sun: u32) -> bool {
    return sun < baked_aux[AUX_SUN_MAP_COUNT];
}

fn sun_map_row(record: u32, row: u32) -> vec4<f32> {
    let base = record + row * 4u;
    let bits = vec4(baked_aux[base], baked_aux[base + 1u], baked_aux[base + 2u], baked_aux[base + 3u]);
    return bitcast<vec4<f32>>(bits);
}

// 1 when a receiver at map `depth` is in front of the surface stored at texel `grid` (clamped
// to the map), or nothing is stored there.
fn sun_map_lit(texels: u32, resolution: u32, grid: vec2<f32>, depth: f32) -> f32 {
    let texel = vec2<u32>(clamp(grid, vec2(0.0), vec2(f32(resolution - 1u))));
    let index = texel.y * resolution + texel.x;
    let pair = unpack2x16unorm(baked_aux[texels + (index >> 1u)]);
    let stored = select(pair.x, pair.y, (index & 1u) == 1u);
    return select(0.0, 1.0, depth <= stored || stored >= 1.0);
}

// The weight of texel `i` of four (from one before the bilinear footprint to one after) in four
// overlapping 3×3 averages blended bilinearly at fraction `w`, the shape of the baked shadows'
// medium and high filters, times 3.
fn sun_map_weight(i: u32, w: f32) -> f32 {
    return select(select(1.0, w, i == 3u), 1.0 - w, i == 0u);
}

// The share of the `sun`-th baked sun's light that static geometry lets through to `world`:
// depth tests of the 4×4 texels around it, weighted like the baked shadows' 3×3 averages
// blended bilinearly. 1 off the map, where no static surface lies along the rays.
fn sun_map_shadow(sun: u32, world: vec3<f32>) -> f32 {
    let record = baked_aux[AUX_SUN_MAPS] + sun * SUN_MAP_RECORD_WORDS;
    let rx = sun_map_row(record, 0u);
    let ry = sun_map_row(record, 1u);
    let rz = sun_map_row(record, 2u);
    let map = vec3(dot(rx.xyz, world) + rx.w, dot(ry.xyz, world) + ry.w, dot(rz.xyz, world) + rz.w);
    if any(map.xy < vec2(0.0)) || any(map.xy > vec2(1.0)) {
        return 1.0;
    }
    let resolution = baked_aux[record + SUN_MAP_RESOLUTION];
    let texels = baked_aux[record + SUN_MAP_TEXELS];
    let size = f32(resolution);
    // Meters to map depth: |rz.xyz| is one over the depth range; a texel spans 1 / (|rx.xyz| size).
    let texel_meters = 1.0 / max(length(rx.xyz) * size, 1.0e-20);
    let depth = map.z - (SUN_MAP_BIAS + SUN_MAP_TEXEL_BIAS * texel_meters) * length(rz.xyz);
    let grid = map.xy * size - 0.5;
    let base = floor(grid);
    let w = grid - base;
    var lit = 0.0;
    for (var y = 0u; y < 4u; y++) {
        var row = 0.0;
        for (var x = 0u; x < 4u; x++) {
            let texel = base + vec2(f32(x), f32(y)) - 1.0;
            row += sun_map_weight(x, w.x) * sun_map_lit(texels, resolution, texel, depth);
        }
        lit += sun_map_weight(y, w.y) * row;
    }
    return lit / 9.0;
}
