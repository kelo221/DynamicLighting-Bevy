// Baked distance cubes as coarse shadow maps for meshes without bake data (Henry's
// `sample_distance_cube_*`, `S/DynamicLighting.cginc` 384–566).
//
// Each baked light has a 64² × 6 cube of closest-hit distances along rays leaving the light,
// two f16 per word (texel 2i low, 2i + 1 high). Face order and orientation follow WebGPU, see
// `shared/cube.rs`; the functions below mirror it. Instead of Henry's index lookup texture the
// texel index is computed directly.

import dynlight::baked_lighting::shaders::baked_data::{
    baked_aux, aux_flags, AUX_CUBES, AUX_CUBE_COUNT, AUX_FLAG_CUBE_ANGULAR,
};

const CUBE_RESOLUTION: u32 = 64u;
// 64 · 64 · 6 texels, two per word.
const CUBE_WORDS: u32 = 12288u;
// Henry's grid for the blurry `DistanceCubes` filter, in meters.
const CUBE_GRID: f32 = 0.25;

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
    let face_uv = direction_to_face_uv(d);
    let texel = min(vec2<u32>(max(face_uv.uv * f32(CUBE_RESOLUTION), vec2(0.0))), vec2(CUBE_RESOLUTION - 1u));
    return face_uv.face * CUBE_RESOLUTION * CUBE_RESOLUTION + texel.y * CUBE_RESOLUTION + texel.x;
}

fn cube_available(slot: u32) -> bool {
    return slot < baked_aux[AUX_CUBE_COUNT];
}

fn cube_distance(slot: u32, index: u32) -> f32 {
    let packed = baked_aux[baked_aux[AUX_CUBES] + slot * CUBE_WORDS + (index >> 1u)];
    let pair = unpack2x16float(packed);
    return select(pair.x, pair.y, (index & 1u) == 1u);
}

// 1 when `world` is closer to the light than the stored hit (the bias is baked in).
fn cube_lit(slot: u32, world: vec3<f32>, light_position: vec3<f32>) -> f32 {
    let outward = world - light_position;
    let stored = cube_distance(slot, direction_to_index(outward));
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
