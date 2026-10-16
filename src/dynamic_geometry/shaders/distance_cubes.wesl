// Baked distance cubes as coarse shadow maps for meshes without bake data (Henry's
// `sample_distance_cube_*`, `S/DynamicLighting.cginc` 384–566).
//
// Each baked light has a 64² × 6 cube of closest-hit distances along rays leaving the light,
// two f16 per word (texel 2i low, 2i + 1 high). Face order and orientation follow WebGPU, see
// `shared/cube.rs`; the functions below mirror it. Instead of Henry's index lookup texture the
// texel index is computed directly. Area lights' cubes are baked from their center and read
// with `cube_area_shadow`.

import dynlight::baked_lighting::shaders::baked_data::{
    baked_aux, aux_flags, AUX_CUBES, AUX_CUBE_COUNT, AUX_FLAG_CUBE_ANGULAR,
};

const CUBE_RESOLUTION: u32 = 64u;
// 64 · 64 · 6 texels, two per word.
const CUBE_WORDS: u32 = 12288u;
// Henry's grid for the blurry `DistanceCubes` filter and the bias it adds, in meters.
const CUBE_GRID: f32 = 0.25;
const CUBE_BIAS: f32 = 0.25;
// The soft lookup of area lights (`dynamic_geometry::mirror`): bilinear taps of the blocker
// search (and its reach, also the widest penumbra, relative to the shape) and of the filter, the
// smallest spread (half a texel, as a tangent), the fractional part of the golden ratio, and the
// coarsest filter grid (`log2` of its cell in texels).
const BLOCKER_TAPS: u32 = 8u;
const SEARCH_SCALE: f32 = 3.0;
const FILTER_TAPS: u32 = 8u;
const TEXEL_SPREAD: f32 = 0.015625;
const GOLDEN_FRACTION: f32 = 0.618034;
const MAX_FILTER_LEVEL: f32 = 4.0;

struct FaceUv {
    face: u32,
    uv: vec2<f32>,
}

// `shared::cube::direction_to_face_uv`.
fn direction_to_face_uv(d: vec3<f32>) -> FaceUv {
    let a = abs(d);
    var face: u32;
    var sc: f32;
    var tc: f32;
    var ma: f32;
    if a.x > a.y && a.x > a.z {
        ma = a.x;
        tc = -d.y;
        if d.x > 0.0 {
            face = 0u;
            sc = -d.z;
        } else {
            face = 1u;
            sc = d.z;
        }
    } else if a.y > a.z {
        ma = a.y;
        sc = d.x;
        if d.y > 0.0 {
            face = 2u;
            tc = d.z;
        } else {
            face = 3u;
            tc = -d.z;
        }
    } else {
        ma = a.z;
        tc = -d.y;
        if d.z > 0.0 {
            face = 4u;
            sc = d.x;
        } else {
            face = 5u;
            sc = -d.x;
        }
    }
    if ma == 0.0 {
        return FaceUv(face, vec2(0.5));
    }
    return FaceUv(face, vec2(sc, tc) / ma * 0.5 + 0.5);
}

// `shared::cube::face_uv_to_direction`, unnormalized. `uv` may leave [0, 1]² slightly.
fn face_uv_to_direction(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let s = uv.x * 2.0 - 1.0;
    let t = uv.y * 2.0 - 1.0;
    switch face {
        case 0u: { return vec3(1.0, -t, -s); }
        case 1u: { return vec3(-1.0, -t, s); }
        case 2u: { return vec3(s, 1.0, t); }
        case 3u: { return vec3(s, -1.0, -t); }
        case 4u: { return vec3(s, -t, 1.0); }
        default: { return vec3(-s, -t, -1.0); }
    }
}

// The flat texel index (`face · 64² + y · 64 + x`) a direction falls in.
fn direction_to_index(d: vec3<f32>) -> u32 {
    return cube_texel(d).index;
}

// The texel a direction falls in: its flat index and the ray through its center.
struct CubeTexel {
    index: u32,
    ray: vec3<f32>,
}

fn cube_texel(d: vec3<f32>) -> CubeTexel {
    let face_uv = direction_to_face_uv(d);
    let texel = min(vec2<u32>(max(face_uv.uv * f32(CUBE_RESOLUTION), vec2(0.0))), vec2(CUBE_RESOLUTION - 1u));
    let index = face_uv.face * CUBE_RESOLUTION * CUBE_RESOLUTION + texel.y * CUBE_RESOLUTION + texel.x;
    let ray = face_uv_to_direction(face_uv.face, (vec2<f32>(texel) + 0.5) / f32(CUBE_RESOLUTION));
    return CubeTexel(index, ray);
}

fn cube_available(slot: u32) -> bool {
    return slot < baked_aux[AUX_CUBE_COUNT];
}

fn cube_distance(slot: u32, index: u32) -> f32 {
    let packed = baked_aux[baked_aux[AUX_CUBES] + slot * CUBE_WORDS + (index >> 1u)];
    let pair = unpack2x16float(packed);
    return select(pair.x, pair.y, (index & 1u) == 1u);
}

// 1 when `world` is closer to the light than the stored hit plus the bias.
fn cube_lit(slot: u32, world: vec3<f32>, light_position: vec3<f32>) -> f32 {
    let outward = world - light_position;
    let stored = cube_distance(slot, direction_to_index(outward)) + CUBE_BIAS;
    return select(0.0, 1.0, dot(outward, outward) <= stored * stored);
}

// `DistanceCubes` mode: eight lookups on a 0.25 m world grid, blended trilinearly.
fn cube_trilinear(slot: u32, world: vec3<f32>, light_position: vec3<f32>) -> f32 {
    let grid = world / CUBE_GRID;
    let w = fract(grid);
    let base = floor(grid) * CUBE_GRID;
    let g = CUBE_GRID;
    let front = vec4(
        cube_lit(slot, base, light_position),
        cube_lit(slot, base + vec3(g, 0.0, 0.0), light_position),
        cube_lit(slot, base + vec3(0.0, g, 0.0), light_position),
        cube_lit(slot, base + vec3(g, g, 0.0), light_position),
    );
    let back = vec4(
        cube_lit(slot, base + vec3(0.0, 0.0, g), light_position),
        cube_lit(slot, base + vec3(g, 0.0, g), light_position),
        cube_lit(slot, base + vec3(0.0, g, g), light_position),
        cube_lit(slot, base + vec3(g, g, g), light_position),
    );
    let z = mix(front, back, w.z);
    let y = mix(z.xy, z.zw, w.y);
    return mix(y.x, y.y, w.x);
}

// `Angular` mode (Henry's angular wedge interpolation): the four texel rays around the light
// direction are each tested against the fragment, both by distance and by the distance to the
// surface plane along that ray, and blended bilinearly. Penumbras widen with distance.
// `direction` points from the fragment to the light; `ndotl` is the raw cosine.
//
// Copyright (c) 2025, Henry de Jongh. All rights reserved.
// This code is licensed under the MIT License.
//
fn cube_angular(slot: u32, distance_sqr: f32, direction: vec3<f32>, normal: vec3<f32>, ndotl: f32) -> f32 {
    let face_uv = direction_to_face_uv(-direction);
    let grid = face_uv.uv * f32(CUBE_RESOLUTION) - 0.5;
    let center = floor(grid) + 0.5;
    let weight = fract(grid);
    let distance = sqrt(distance_sqr);
    let bias = 0.05 + 0.05 * (1.0 - ndotl);
    let facing = max(0.001, ndotl);
    // Spherical test for grazing angles, planar test elsewhere.
    let t = pow(saturate(facing * 10.0), 4.0);
    let projected = distance * facing;
    var lit = 0.0;
    for (var y = 0u; y < 2u; y++) {
        for (var x = 0u; x < 2u; x++) {
            let uv = (center + vec2(f32(x), f32(y))) / f32(CUBE_RESOLUTION);
            let ray = normalize(face_uv_to_direction(face_uv.face, uv));
            let stored = cube_distance(slot, direction_to_index(ray)) + bias;
            let plane = projected / max(0.001, dot(normal, -ray));
            let tests = step(vec2(distance, plane), vec2(stored));
            let weights = select(1.0 - weight, weight, vec2(x == 1u, y == 1u));
            lit += mix(tests.x, tests.y, t) * weights.x * weights.y;
        }
    }
    return lit;
}

// The baked shadow term of light `slot` for dynamic geometry, using the mode the cubes were
// baked for. 1 when the light has no cube.
fn cube_shadow(slot: u32, world: vec3<f32>, light_position: vec3<f32>, distance_sqr: f32, direction: vec3<f32>, normal: vec3<f32>) -> f32 {
    if !cube_available(slot) {
        return 1.0;
    }
    if (aux_flags() & AUX_FLAG_CUBE_ANGULAR) != 0u {
        return cube_angular(slot, distance_sqr, direction, normal, dot(normal, direction));
    }
    return cube_trilinear(slot, world, light_position);
}

// Soft shadows of rectangle and tube lights (`dynamic_geometry::mirror`). A light point offset
// by `o` from the center sees a blocker a fraction `s` of the way to the receiver along the
// receiver's direction `d` where the center sees `d + o · (1 − s) / s`. A blocker search over
// the shape finds the mean fraction (each blocker's distance over the receiver plane's along
// the same ray), then a filter over the shape scaled by `(1 − s) / s` gives the share of the
// light reaching the receiver. `radius` is the light's radius around its center, which the
// cube stores where rays hit nothing. Every lookup blends the depth tests of four
// texels bilinearly, like the angular filter, so the result changes smoothly as the receiver
// moves. Filter taps read a grid as wide as their spacing, so each tap's ramp meets the next;
// the grids are fixed per level like mips (the centers of 1, 2, 4, … texel wide blocks, which
// line up across face seams), and a tap blends the two levels around the spacing. Each grid
// point tests the texel it falls in along that texel's own ray.
fn cube_area_shadow(slot: u32, world: vec3<f32>, normal: vec3<f32>, center: vec3<f32>, axis_x: vec3<f32>, axis_y: vec3<f32>, half: vec2<f32>, radius: f32) -> f32 {
    if !cube_available(slot) {
        return 1.0;
    }
    let outward = world - center;
    let distance = length(outward);
    if distance < 1.0e-4 {
        return 1.0;
    }
    let ndotl = dot(normal, -outward / distance);
    let facing = max(0.001, ndotl);
    let test = DepthTest(distance, distance * facing, pow(saturate(facing * 10.0), 4.0), 0.05 + 0.05 * (1.0 - ndotl), radius);
    let minimum = vec2(distance * TEXEL_SPREAD);

    // The blocked share of the search and the blockers' fraction of the way times it, bilinear.
    var spread = max(half * SEARCH_SCALE, minimum);
    var blocked = 0.0;
    var fraction = 0.0;
    for (var k = 0u; k < BLOCKER_TAPS; k++) {
        let tap = lattice_tap(k, BLOCKER_TAPS);
        let direction = outward + axis_x * (tap.x * spread.x) + axis_y * (tap.y * spread.y);
        let lookup = bilinear_lookup(slot, test, normal, direction, 1.0);
        blocked += 1.0 - lookup.x;
        fraction += lookup.y;
    }
    if blocked <= 0.0 {
        return 1.0;
    }
    fraction /= blocked;
    let scale = clamp((1.0 - fraction) / max(fraction, 1.0e-3), 0.0, SEARCH_SCALE);
    spread = max(half * scale, minimum);
    let a = abs(outward);
    let major = max(a.x, max(a.y, a.z));
    // The taps' spacing: along a tube, or as squares sharing a rectangle's area.
    let taps = f32(FILTER_TAPS);
    let spacing = max(max(spread.x, spread.y) / taps, sqrt(spread.x * spread.y / taps));
    let texels = spacing / major * f32(CUBE_RESOLUTION);
    let level = min(log2(max(texels, 1.0)), MAX_FILTER_LEVEL);
    let fine = floor(level);
    let blend = level - fine;
    let step = f32(1u << u32(fine));
    // Each tap on the finer level, then the coarser one; one lookup call keeps the shader small.
    var lit = 0.0;
    for (var k = 0u; k < 2u * FILTER_TAPS; k++) {
        let coarse = (k & 1u) == 1u;
        let weight = select(1.0 - blend, blend, coarse);
        if weight <= 0.0 {
            continue;
        }
        let tap = lattice_tap(k >> 1u, FILTER_TAPS);
        let direction = outward + axis_x * (tap.x * spread.x) + axis_y * (tap.y * spread.y);
        lit += weight * bilinear_lookup(slot, test, normal, direction, select(step, 2.0 * step, coarse)).x;
    }
    lit /= f32(FILTER_TAPS);
    // Below one tap's worth of blockers, fade to lit as they leave the search.
    return mix(lit, 1.0, 1.0 - min(blocked, 1.0));
}

// The receiver's depth test along a lookup: the stored distance against the distance to the
// receiver's plane along the lookup, or near grazing angles against the receiver's own distance.
// The plane distance stops at the light's radius, which the cube stores for empty space.
struct DepthTest {
    distance: f32,
    projected: f32,
    grazing: f32,
    bias: f32,
    radius: f32,
}

// Whether a stored distance along `direction` lets the light through (0 or 1), and the
// receiver's distance along it that the test compares with.
fn depth_lit(test: DepthTest, normal: vec3<f32>, direction: vec3<f32>, stored: f32) -> vec2<f32> {
    let ray = normalize(direction);
    let plane = min(test.projected / max(0.001, dot(normal, -ray)), test.radius);
    let tests = step(vec2(test.distance, plane), vec2(stored + test.bias));
    return vec2(mix(tests.x, tests.y, test.grazing), mix(test.distance, plane, test.grazing));
}

// The depth tests at the four centers of `step` texel wide blocks around `direction`, blended
// bilinearly: the lit share, and the blocked share times the blocker's fraction of the way to
// the receiver. A step of one reads the four nearest texels.
fn bilinear_lookup(slot: u32, test: DepthTest, normal: vec3<f32>, direction: vec3<f32>, step: f32) -> vec2<f32> {
    let face_uv = direction_to_face_uv(direction);
    let grid = face_uv.uv * (f32(CUBE_RESOLUTION) / step) - 0.5;
    let corner = floor(grid);
    let weight = grid - corner;
    var sum = vec2(0.0);
    for (var y = 0u; y < 2u; y++) {
        for (var x = 0u; x < 2u; x++) {
            let uv = (corner + vec2(f32(x), f32(y)) + 0.5) * step / f32(CUBE_RESOLUTION);
            let texel = cube_texel(face_uv_to_direction(face_uv.face, uv));
            let weights = select(1.0 - weight, weight, vec2(x == 1u, y == 1u));
            let stored = cube_distance(slot, texel.index);
            let tested = depth_lit(test, normal, texel.ray, stored);
            sum += vec2(tested.x, (1.0 - tested.x) * stored / tested.y) * (weights.x * weights.y);
        }
    }
    return sum;
}

// Tap `k` of `n` over the square [−1, 1]²: evenly spaced across x, spread across y by the
// golden ratio (a Fibonacci lattice), so taps cover a rectangle uniformly and a tube evenly.
fn lattice_tap(k: u32, n: u32) -> vec2<f32> {
    let x = (f32(k) + 0.5) / f32(n);
    let y = fract(f32(k) * GOLDEN_FRACTION + 0.5);
    return vec2(x, y) * 2.0 - 1.0;
}
