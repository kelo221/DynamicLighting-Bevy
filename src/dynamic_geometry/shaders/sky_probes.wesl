// Baked sky probes as the sky's shadows for meshes without bake data: the sky's counterpart of
// the sun maps. Mirrors `dynamic_geometry::sky_mirror`.
//
// The probes form a grid over the static scene, x fastest, then y, then z. Each holds the
// openness of the hemispheres around +X, −X, +Y and −Y in the four bytes of its first word and
// around +Z and −Z in the low two of its second, 255 for open sky. The record holds the grid's
// origin, spacing, probe counts and first probe word.

import dynlight::baked_lighting::shaders::baked_data::{
    baked_aux, AUX_SKY_PROBES, SKY_PROBE_SPACING, SKY_PROBE_COUNTS, SKY_PROBE_WORDS,
};

// The two words of every probe.
const SKY_PROBE_STRIDE: u32 = 2u;

// How much of the sky a surface at `world` facing unit `normal` sees, 0 to 1: the probes'
// openness blended trilinearly, clamped to the grid, then weighted by the squared normal like an
// ambient cube. 1 without probes.
fn sky_probe_openness(world: vec3<f32>, normal: vec3<f32>) -> f32 {
    let record = baked_aux[AUX_SKY_PROBES];
    if record == 0u {
        return 1.0;
    }
    let origin = bitcast<vec3<f32>>(vec3(baked_aux[record], baked_aux[record + 1u], baked_aux[record + 2u]));
    let spacing = bitcast<f32>(baked_aux[record + SKY_PROBE_SPACING]);
    let counts = vec3(
        baked_aux[record + SKY_PROBE_COUNTS],
        baked_aux[record + SKY_PROBE_COUNTS + 1u],
        baked_aux[record + SKY_PROBE_COUNTS + 2u],
    );
    let probes = baked_aux[record + SKY_PROBE_WORDS];
    let last = counts - 1u;
    let grid = clamp((world - origin) / spacing, vec3(0.0), vec3<f32>(last));
    let base = min(vec3<u32>(grid), max(counts, vec3(2u)) - 2u);
    let f = grid - vec3<f32>(base);
    var axes_xy = vec4(0.0);
    var axes_z = vec2(0.0);
    for (var corner = 0u; corner < 8u; corner++) {
        let step = vec3(corner & 1u, (corner >> 1u) & 1u, corner >> 2u);
        let probe = min(base + step, last);
        let w = select(1.0 - f, f, step == vec3(1u));
        let weight = w.x * w.y * w.z;
        let index = probes + ((probe.z * counts.y + probe.y) * counts.x + probe.x) * SKY_PROBE_STRIDE;
        axes_xy += weight * unpack4x8unorm(baked_aux[index]);
        axes_z += weight * unpack4x8unorm(baked_aux[index + 1u]).xy;
    }
    let along = vec3(
        select(axes_xy.y, axes_xy.x, normal.x > 0.0),
        select(axes_xy.w, axes_xy.z, normal.y > 0.0),
        select(axes_z.y, axes_z.x, normal.z > 0.0),
    );
    return dot(normal * normal, along);
}
