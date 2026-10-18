// Henry's `DynamicTriangle` (`S/DynamicLighting.cginc` 608–962): a triangle's baked light list,
// its shadows with the four quality tiers, and its bounce texels.
//
// Shadow-map coordinates are texels: UV1 · receiver resolution. Texel `x + y · width` of a
// light's shadow data is texel `(x, y) − bounds.xy` of the triangle's padded crop rectangle; the
// subtraction wraps like Henry's `uint` math. Shadow texels are Henry's single bits, or 2 or 4
// bits of visibility for area lights and suns with a disk, or 8 bits for the sky, `32 / bits` per
// word from the low bits up.

import dynlight::baked_lighting::shaders::baked_data::{
    baked_triangles, Receiver, aux_flags, AUX_FLAG_BOUNCE_LAYOUT,
};

// Henry clamps garbage light counts so a bad buffer can't hang the GPU.
const MAX_TRIANGLE_LIGHTS: u32 = 256u;

const QUALITY_INTEGRATED: u32 = 0u;
const QUALITY_LOW: u32 = 1u;
const QUALITY_MEDIUM: u32 = 2u;
const QUALITY_HIGH: u32 = 3u;

struct Triangle {
    // Absolute word index of the receiver's data.
    base: u32,
    // `(minX − 2, minY − 2, padded width)` of the crop rectangle.
    bounds: vec3<u32>,
    // Absolute word index of the first light entry.
    lights: u32,
    light_count: u32,
    // Words per light entry: 2, or 3 with bounce offsets.
    stride: u32,
}

struct TriangleLight {
    // Slot in the light buffer.
    index: u32,
    // Absolute word index of the shadow texels; 0 when the light reaches the whole triangle.
    shadow: u32,
    // Absolute word index of the bounce texels; 0 without bounce data.
    bounce: u32,
}

fn absolute(base: u32, offset: u32) -> u32 {
    return select(0u, base + offset, offset != 0u);
}

// `DynamicTriangle.load`. Primitives the bake doesn't know get no baked lights.
fn load_triangle(receiver: Receiver, primitive: u32) -> Triangle {
    var triangle = Triangle(receiver.triangles_base, vec3(0u), 0u, 0u, 2u);
    if primitive >= receiver.triangle_count {
        return triangle;
    }
    let header = receiver.triangles_base + primitive * 4u;
    let light_data = receiver.triangles_base + baked_triangles[header];
    triangle.bounds = vec3(
        baked_triangles[header + 1u],
        baked_triangles[header + 2u],
        baked_triangles[header + 3u],
    );
    triangle.light_count = min(baked_triangles[light_data], MAX_TRIANGLE_LIGHTS);
    triangle.lights = light_data + 1u;
    triangle.stride = select(2u, 3u, (aux_flags() & AUX_FLAG_BOUNCE_LAYOUT) != 0u);
    return triangle;
}

// `DynamicTriangle.set_active_light_index` for `k < light_count`.
fn triangle_light(triangle: Triangle, k: u32) -> TriangleLight {
    let entry = triangle.lights + k * triangle.stride;
    var bounce = 0u;
    if triangle.stride == 3u {
        bounce = baked_triangles[entry + 2u];
    }
    return TriangleLight(
        baked_triangles[entry],
        absolute(triangle.base, baked_triangles[entry + 1u]),
        absolute(triangle.base, bounce),
    );
}

// 0, 1, 2 or 3 for 1, 2, 4 or 8 shadow bits per texel (the light's channel bits 20–22).
fn shadow_log_bits(bits_per_texel: u32) -> u32 {
    return min(firstLeadingBit(max(bits_per_texel, 1u)), 3u);
}

// Eight shadow texels from grid texel `uv` rightward, `1 << log_bits` bits each, or four at 8
// bits: texel k of the row is texel `uv + (k, 0)`. As in Henry's `x + y · width` indexing, a run
// that leaves the grid sideways continues in the neighbouring row. Rows above the grid read its
// first row and texels before the first read the first texel, where Henry's wrapping index read
// outside the data. At most two loads.
fn shadow_row(triangle: Triangle, entry: TriangleLight, uv: vec2<i32>, log_bits: u32) -> u32 {
    let bits = 1u << log_bits;
    let span = min(8u, 32u >> log_bits);
    let word_log = 5u - log_bits;
    let start = max(uv.y, 0) * i32(triangle.bounds.z) + uv.x;
    let index = u32(max(start, 0));
    let word = entry.shadow + (index >> word_log);
    let shift = (index & ((1u << word_log) - 1u)) << log_bits;
    var row = baked_triangles[word] >> shift;
    if shift + span * bits > 32u {
        row |= baked_triangles[word + 1u] << (32u - shift);
    }
    if start < 0 {
        let top = (1u << bits) - 1u;
        let before = min(u32(-start), span) << log_bits;
        let fill = (0xffffffffu / top) >> (32u - before);
        row = select(0u, row << before, before < 32u) | ((row & top) * fill);
    }
    return select(row, row & ((1u << (8u * bits)) - 1u), bits < 4u);
}

// The level of texel `x` of a row from `shadow_row`: 0 (shadowed) to `2^bits - 1` (lit).
fn row_level(row: u32, x: u32, log_bits: u32) -> f32 {
    let top = (1u << (1u << log_bits)) - 1u;
    return f32((row >> (x << log_bits)) & top);
}

// The levels of texels 0 to 3 of a row.
fn row_levels(row: u32, log_bits: u32) -> vec4<f32> {
    return vec4(
        row_level(row, 0u, log_bits),
        row_level(row, 1u, log_bits),
        row_level(row, 2u, log_bits),
        row_level(row, 3u, log_bits),
    );
}

// The 1 4 6 4 1 binomial sums of texels `x` to `x + 4` of eight texels from `uv` rightward, for
// x = 0 to 3. Texels 4 to 7 come from the same row read, or at 8 bits from a second one.
fn row_binomials(triangle: Triangle, entry: TriangleLight, uv: vec2<i32>, log_bits: u32) -> vec4<f32> {
    let row = shadow_row(triangle, entry, uv, log_bits);
    var rest = row >> ((4u << log_bits) & 31u);
    if log_bits == 3u {
        rest = shadow_row(triangle, entry, uv + vec2(4, 0), log_bits);
    }
    let low = row_levels(row, log_bits);
    let high = row_levels(rest, log_bits);
    let side = low + high;
    let near = vec4(low.yzw, high.x) + vec4(low.w, high.xyz);
    return side + 4.0 * near + 6.0 * vec4(low.zw, high.xy);
}

// The binomial sum of five rows of `row_binomials`, down.
fn column_binomials(a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>, e: vec4<f32>) -> vec4<f32> {
    let side = a + e;
    let near = b + d;
    return (side + 4.0 * near + 6.0 * c) / 256.0;
}

// Where the filters read around texel coordinates `pixel`; the same for every light of the
// triangle, so the lighting loop computes it once.
struct Footprint {
    // Bilinear weights.
    f: vec2<f32>,
    // The grid texel at the top-left of the bilinear footprint.
    corner: vec2<u32>,
    // The grid texel containing `pixel`.
    texel: vec2<u32>,
}

fn footprint(triangle: Triangle, pixel: vec2<f32>) -> Footprint {
    return Footprint(
        fract(pixel - 0.5),
        vec2<u32>(max(floor(pixel - 0.5), vec2(0.0))) - triangle.bounds.xy,
        vec2<u32>(max(pixel, vec2(0.0))) - triangle.bounds.xy,
    );
}

// Integrated graphics: bilinear over four texels. The filters return levels; `baked_shadow`
// scales them to 0–1.
fn shadow_integrated(triangle: Triangle, entry: TriangleLight, at: Footprint, log_bits: u32) -> f32 {
    let f = at.f;
    let corner = vec2<i32>(at.corner);
    let top = shadow_row(triangle, entry, corner, log_bits);
    let bottom = shadow_row(triangle, entry, corner + vec2(0, 1), log_bits);
    let left = vec2(row_level(top, 0u, log_bits), row_level(bottom, 0u, log_bits));
    let right = vec2(row_level(top, 1u, log_bits), row_level(bottom, 1u, log_bits));
    let c = mix(left, right, f.x);
    return mix(c.x, c.y, f.y);
}

// Low: the average of the 3×3 texels around the texel containing `pixel`.
fn shadow_3x3(triangle: Triangle, entry: TriangleLight, at: Footprint, log_bits: u32) -> f32 {
    let uv = vec2<i32>(at.texel) - vec2(1);
    var sum = 0.0;
    for (var y = 0; y < 3; y++) {
        let row = shadow_row(triangle, entry, uv + vec2(0, y), log_bits);
        sum += row_level(row, 0u, log_bits) + row_level(row, 1u, log_bits)
            + row_level(row, 2u, log_bits);
    }
    return sum / 9.0;
}

// The 3×3 windows of a 4×4 block of one-bit texels, texel (x, y) at bit 4y + x.
const WINDOW_TL: u32 = 0x0777u;
const WINDOW_TR: u32 = 0x0EEEu;
const WINDOW_BL: u32 = 0x7770u;
const WINDOW_BR: u32 = 0xEEE0u;

// Medium for one-bit shadows: the same four 3×3 sums as `shadow_bilinear`, counted as bits.
fn shadow_bilinear_bits(triangle: Triangle, entry: TriangleLight, at: Footprint) -> f32 {
    let f = at.f;
    let corner = vec2<i32>(at.corner);
    var block = 0u;
    for (var y = 0u; y < 4u; y++) {
        let row = shadow_row(triangle, entry, corner + vec2(-1, i32(y) - 1), 0u);
        block |= (row & 0xFu) << (4u * y);
    }
    let tl = f32(countOneBits(block & WINDOW_TL));
    let tr = f32(countOneBits(block & WINDOW_TR));
    let bl = f32(countOneBits(block & WINDOW_BL));
    let br = f32(countOneBits(block & WINDOW_BR));
    let c = mix(vec2(tl, bl), vec2(tr, br), f.x);
    return mix(c.x, c.y, f.y) / 9.0;
}

// Medium and high: four overlapping 3×3 averages, blended bilinearly. High reads each of the
// 4×4 texels through the 5×5 Gaussian (binomial weights, applied across then down).
fn shadow_bilinear(triangle: Triangle, entry: TriangleLight, at: Footprint, gaussian: bool, log_bits: u32) -> f32 {
    if !gaussian && log_bits == 0u {
        return shadow_bilinear_bits(triangle, entry, at);
    }
    let corner = vec2<i32>(at.corner);
    if gaussian {
        // Rows −3 to +4 around the corner, from column −3.
        let r = corner + vec2(-3, -3);
        let a0 = row_binomials(triangle, entry, r, log_bits);
        let a1 = row_binomials(triangle, entry, r + vec2(0, 1), log_bits);
        let a2 = row_binomials(triangle, entry, r + vec2(0, 2), log_bits);
        let a3 = row_binomials(triangle, entry, r + vec2(0, 3), log_bits);
        let a4 = row_binomials(triangle, entry, r + vec2(0, 4), log_bits);
        let a5 = row_binomials(triangle, entry, r + vec2(0, 5), log_bits);
        let a6 = row_binomials(triangle, entry, r + vec2(0, 6), log_bits);
        let a7 = row_binomials(triangle, entry, r + vec2(0, 7), log_bits);
        return blend_rows(
            column_binomials(a0, a1, a2, a3, a4),
            column_binomials(a1, a2, a3, a4, a5),
            column_binomials(a2, a3, a4, a5, a6),
            column_binomials(a3, a4, a5, a6, a7),
            at.f,
        );
    }
    let r = corner + vec2(-1, -1);
    return blend_rows(
        row_levels(shadow_row(triangle, entry, r, log_bits), log_bits),
        row_levels(shadow_row(triangle, entry, r + vec2(0, 1), log_bits), log_bits),
        row_levels(shadow_row(triangle, entry, r + vec2(0, 2), log_bits), log_bits),
        row_levels(shadow_row(triangle, entry, r + vec2(0, 3), log_bits), log_bits),
        at.f,
    );
}

// The four overlapping 3×3 sums of a 4×4 map given as rows, blended bilinearly, over 9.
fn blend_rows(m0: vec4<f32>, m1: vec4<f32>, m2: vec4<f32>, m3: vec4<f32>, f: vec2<f32>) -> f32 {
    let middle = m1.y + m1.z + m2.y + m2.z;
    let top = middle + m0.y + m0.z;
    let tl = top + m0.x + m1.x + m2.x;
    let tr = top + m0.w + m1.w + m2.w;
    let bottom = middle + m3.y + m3.z;
    let bl = bottom + m1.x + m2.x + m3.x;
    let br = bottom + m1.w + m2.w + m3.w;
    let c = mix(vec2(tl, bl), vec2(tr, br), f.x);
    return mix(c.x, c.y, f.y) / 9.0;
}

// How much of the light reaches the footprint's texel coordinates, 0 to 1, at the given quality,
// for a light with `bits_per_texel` shadow bits.
fn baked_shadow(triangle: Triangle, entry: TriangleLight, at: Footprint, quality: u32, bits_per_texel: u32) -> f32 {
    if entry.shadow == 0u {
        return 1.0;
    }
    let log_bits = shadow_log_bits(bits_per_texel);
    var levels: f32;
    if quality == QUALITY_INTEGRATED {
        levels = shadow_integrated(triangle, entry, at, log_bits);
    } else if quality == QUALITY_LOW {
        levels = shadow_3x3(triangle, entry, at, log_bits);
    } else {
        levels = shadow_bilinear(triangle, entry, at, quality >= QUALITY_HIGH, log_bits);
    }
    return levels / f32((1u << (1u << log_bits)) - 1u);
}

// One stored bounce texel by grid index, 0 to 1 (still square-rooted).
fn bounce_texel(entry: TriangleLight, index: u32, bits: u32) -> f32 {
    let pixels = 32u / bits;
    let mask = (1u << bits) - 1u;
    let word = baked_triangles[entry.bounce + index / pixels];
    return f32((word >> ((index % pixels) * bits)) & mask) / f32(mask);
}

// Bilinear bounce light at the footprint, squared back to linear. `bits` is the light's bounce
// bits per pixel.
fn baked_bounce(triangle: Triangle, entry: TriangleLight, at: Footprint, bits: u32) -> f32 {
    if entry.bounce == 0u {
        return 0.0;
    }
    let f = at.f;
    let corner = at.corner;
    let top = corner.y * triangle.bounds.z + corner.x;
    let bottom = top + triangle.bounds.z;
    let tl = bounce_texel(entry, top, bits);
    let tr = bounce_texel(entry, top + 1u, bits);
    let bl = bounce_texel(entry, bottom, bits);
    let br = bounce_texel(entry, bottom + 1u, bits);
    let c = mix(vec2(tl, bl), vec2(tr, br), f.x);
    let value = mix(c.x, c.y, f.y);
    return value * value;
}
