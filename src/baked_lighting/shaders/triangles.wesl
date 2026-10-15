// Henry's `DynamicTriangle` (`S/DynamicLighting.cginc` 608–962): a triangle's baked light list,
// its shadow bits with the four quality tiers, and its bounce texels.
//
// Shadow-map coordinates are texels: UV1 · receiver resolution. Bit `x + y · width` of a light's
// shadow data is texel `(x, y) − bounds.xy` of the triangle's padded crop rectangle; the
// subtraction wraps like Henry's `uint` math.

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
    // Absolute word index of the shadow bits; 0 when the light reaches the whole triangle.
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

// One shadow bit by grid index `x + y · width`, as 0 or 1.
fn shadow_index(entry: TriangleLight, index: u32) -> f32 {
    let word = baked_triangles[entry.shadow + index / 32u];
    return f32((word >> (index % 32u)) & 1u);
}

// One shadow bit at grid texel `uv` (already minus `bounds.xy`).
fn shadow_texel(triangle: Triangle, entry: TriangleLight, uv: vec2<u32>) -> f32 {
    return shadow_index(entry, uv.y * triangle.bounds.z + uv.x);
}

// Row `i` of Pascal's triangle for n = 4: 1 4 6 4 1.
fn binomial4(i: u32) -> f32 {
    return select(select(6.0, 4.0, i == 1u || i == 3u), 1.0, i == 0u || i == 4u);
}

// 5×5 Gaussian around grid texel `uv`.
fn shadow_gaussian5(triangle: Triangle, entry: TriangleLight, uv: vec2<u32>) -> f32 {
    var sum = 0.0;
    for (var y = 0u; y < 5u; y++) {
        for (var x = 0u; x < 5u; x++) {
            let texel = uv + vec2(x, y) - vec2(2u);
            sum += binomial4(x) * binomial4(y) * shadow_texel(triangle, entry, texel);
        }
    }
    return sum / 256.0;
}

// The grid texel at the top-left of a bilinear footprint around texel coordinates `pixel`.
fn footprint_corner(triangle: Triangle, pixel: vec2<f32>) -> vec2<u32> {
    return vec2<u32>(max(floor(pixel - 0.5), vec2(0.0))) - triangle.bounds.xy;
}

// Integrated graphics: bilinear over four single bits.
fn shadow_integrated(triangle: Triangle, entry: TriangleLight, pixel: vec2<f32>) -> f32 {
    let f = fract(pixel - 0.5);
    let corner = footprint_corner(triangle, pixel);
    let tl = shadow_texel(triangle, entry, corner);
    let tr = shadow_texel(triangle, entry, corner + vec2(1u, 0u));
    let bl = shadow_texel(triangle, entry, corner + vec2(0u, 1u));
    let br = shadow_texel(triangle, entry, corner + vec2(1u, 1u));
    let c = mix(vec2(tl, bl), vec2(tr, br), f.x);
    return mix(c.x, c.y, f.y);
}

// Low: the average of the 3×3 texels around the texel containing `pixel`.
fn shadow_3x3(triangle: Triangle, entry: TriangleLight, pixel: vec2<f32>) -> f32 {
    let uv = vec2<u32>(max(pixel, vec2(0.0))) - triangle.bounds.xy;
    var sum = 0.0;
    for (var y = 0u; y < 3u; y++) {
        let row = (uv.y + y - 1u) * triangle.bounds.z + uv.x;
        for (var x = 0u; x < 3u; x++) {
            sum += shadow_index(entry, row + x - 1u);
        }
    }
    return sum / 9.0;
}

// Medium and high: four overlapping 3×3 averages, blended bilinearly. High reads each of the
// 4×4 texels through the 5×5 Gaussian.
fn shadow_bilinear(triangle: Triangle, entry: TriangleLight, pixel: vec2<f32>, gaussian: bool) -> f32 {
    let f = fract(pixel - 0.5);
    let corner = footprint_corner(triangle, pixel);
    var map: array<array<f32, 4>, 4>;
    for (var y = 0u; y < 4u; y++) {
        for (var x = 0u; x < 4u; x++) {
            let texel = corner + vec2(x, y) - vec2(1u);
            if gaussian {
                map[y][x] = shadow_gaussian5(triangle, entry, texel);
            } else {
                map[y][x] = shadow_texel(triangle, entry, texel);
            }
        }
    }
    let middle = map[1][1] + map[1][2] + map[2][1] + map[2][2];
    let top = middle + map[0][1] + map[0][2];
    let tl = top + map[0][0] + map[1][0] + map[2][0];
    let tr = top + map[0][3] + map[1][3] + map[2][3];
    let bottom = middle + map[3][1] + map[3][2];
    let bl = bottom + map[1][0] + map[2][0] + map[3][0];
    let br = bottom + map[1][3] + map[2][3] + map[3][3];
    let c = mix(vec2(tl, bl), vec2(tr, br), f.x);
    return mix(c.x, c.y, f.y) / 9.0;
}

// How much of the light reaches texel coordinates `pixel`, 0 to 1, at the given quality.
fn baked_shadow(triangle: Triangle, entry: TriangleLight, pixel: vec2<f32>, quality: u32) -> f32 {
    if entry.shadow == 0u {
        return 1.0;
    }
    if quality == QUALITY_INTEGRATED {
        return shadow_integrated(triangle, entry, pixel);
    }
    if quality == QUALITY_LOW {
        return shadow_3x3(triangle, entry, pixel);
    }
    return shadow_bilinear(triangle, entry, pixel, quality >= QUALITY_HIGH);
}

// One stored bounce texel by grid index, 0 to 1 (still square-rooted).
fn bounce_texel(entry: TriangleLight, index: u32, bits: u32) -> f32 {
    let pixels = 32u / bits;
    let mask = (1u << bits) - 1u;
    let word = baked_triangles[entry.bounce + index / pixels];
    return f32((word >> ((index % pixels) * bits)) & mask) / f32(mask);
}

// Bilinear bounce light at texel coordinates `pixel`, squared back to linear. `bits` is the
// light's bounce bits per pixel.
fn baked_bounce(triangle: Triangle, entry: TriangleLight, pixel: vec2<f32>, bits: u32) -> f32 {
    if entry.bounce == 0u {
        return 0.0;
    }
    let f = fract(pixel - 0.5);
    let corner = footprint_corner(triangle, pixel);
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
