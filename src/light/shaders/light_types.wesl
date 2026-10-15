// Light-type patterns, shimmer and attenuation for one light at one point.
// `dynlight::light::mirror` is the CPU reference for these functions.

import dynlight::shared::shaders::types::{
    GpuLight, LIGHT_TYPE_WAVE, LIGHT_TYPE_INTERFERENCE, LIGHT_TYPE_ROTOR, LIGHT_TYPE_SHOCK,
    LIGHT_TYPE_DISCO, LIGHT_SHIMMER_WATER, LIGHT_SHIMMER_RANDOM, light_type, light_shimmer,
};

const PI: f32 = 3.141592653589793;
const TAU: f32 = 6.283185307179586;
const HALF_PI: f32 = 1.5707963267948966;
// tan(π / 8), the snapping threshold between an axis and a diagonal.
const SNAP_THRESHOLD: f32 = 0.41421356237309503;

// Smooth falloff to zero at the radius: intensity · (1 − s)² / (1 + falloff · s), s = d² / r².
fn attenuation(light: GpuLight, distance_sqr: f32) -> f32 {
    let s = saturate(distance_sqr / light.radius_sqr);
    return light.intensity * (1.0 - s) * (1.0 - s) / (1.0 + light.falloff * s);
}

// `value` in the light's basis (right, up, forward). In Bevy's right-handed space the right axis
// is up × forward.
fn to_light_space(value: vec3<f32>, forward: vec3<f32>, up: vec3<f32>) -> vec3<f32> {
    let right = cross(up, forward);
    return vec3(dot(value, right), dot(value, up), dot(value, forward));
}

// Smooth cone edge between the outer and inner cutoff cosines; equal cutoffs give a hard edge.
fn cone(theta: f32, inner_cos: f32, outer_cos: f32) -> f32 {
    let epsilon = inner_cos - outer_cos;
    if abs(epsilon) <= 1.0e-6 {
        return select(0.0, 1.0, theta >= inner_cos);
    }
    return saturate((theta - outer_cos) / epsilon);
}

fn snap_component(c: f32) -> f32 {
    return select(sign(c), 0.0, abs(c) < SNAP_THRESHOLD);
}

// Snaps a direction to the nearest of the 26 axis and diagonal directions.
fn snap_direction(value: vec3<f32>) -> vec3<f32> {
    let a = abs(value);
    let unit_cube = value / max(max(a.x, a.y), a.z);
    let snapped = vec3(snap_component(unit_cube.x), snap_component(unit_cube.y), snap_component(unit_cube.z));
    return normalize(snapped);
}

// The spot cone cosine for `direction` (normalized, fragment to light).
fn spot_theta(light: GpuLight, direction: vec3<f32>) -> f32 {
    return dot(direction, light.forward);
}

// The discoball beam cosine: angle to the nearest of the 26 beams in light space.
fn discoball_theta(light: GpuLight, direction: vec3<f32>) -> f32 {
    let local = to_light_space(direction, light.forward, light.up);
    return dot(snap_direction(local), local);
}

fn rotor(light: GpuLight, local: vec3<f32>) -> f32 {
    let angle = light.gp2 * atan2(local.x, local.z);
    var scale = 0.5 + 0.5 * cos(angle + light.gp1);
    let center_size = light.radius_sqr * abs(light.gp3);
    var distance_sqr = dot(local.xz, local.xz);
    if light.gp3 < 0.0 {
        if distance_sqr < center_size {
            scale *= pow(distance_sqr / center_size, PI);
        }
    } else {
        distance_sqr /= center_size;
        if distance_sqr < 1.0 {
            scale = 1.0 - distance_sqr + scale * distance_sqr;
        }
    }
    return pow(scale, HALF_PI);
}

fn disco(light: GpuLight, local: vec3<f32>) -> f32 {
    let horizontal = light.gp2 * atan2(local.x, local.z);
    let vertical = light.gp2 * atan2(length(local.xz), local.y);
    let a = 0.5 + 0.5 * cos(horizontal + light.gp1);
    let b = 0.5 + 0.5 * cos(vertical - light.gp3);
    var scale = a + b - a * b;
    let distance = 0.5 * dot(local.xz, local.xz);
    if distance < 1.0 {
        scale *= distance;
    }
    return 1.0 - scale;
}

fn shock(light: GpuLight, distance: f32) -> f32 {
    let d = light.gp2 * distance;
    var brightness = 0.9 + 0.1 * sin((d * 2.0 - light.gp1) * TAU);
    brightness *= 0.9 + 0.1 * cos((d + light.gp1) * TAU);
    return brightness * (0.9 + 0.1 * sin((d * 0.5 - light.gp1) * TAU));
}

// The pattern of the non-cone light types at a point, 0 to 1. `to_light` is the light position
// minus the world position. Point, spot and discoball lights return 1; their cones are applied
// separately because they also gate cookies and bounce.
fn pattern(light: GpuLight, to_light: vec3<f32>) -> f32 {
    switch light_type(light) {
        case LIGHT_TYPE_WAVE: {
            return 0.7 + 0.3 * sin((length(to_light) - light.gp1) * light.gp2);
        }
        case LIGHT_TYPE_INTERFERENCE: {
            let local = to_light_space(to_light, light.forward, light.up);
            let angle = atan2(length(local.xz), local.y) * light.gp2;
            return 0.5 + 0.5 * cos(angle - light.gp1);
        }
        case LIGHT_TYPE_ROTOR: {
            return rotor(light, to_light_space(to_light, light.forward, light.up));
        }
        case LIGHT_TYPE_SHOCK: {
            return shock(light, length(to_light));
        }
        case LIGHT_TYPE_DISCO: {
            return disco(light, to_light_space(to_light, light.forward, light.up));
        }
        default: {
            return 1.0;
        }
    }
}

// lowbias32 integer hash by Chris Wellons (public domain).
fn hash_u32(value: u32) -> u32 {
    var h = value;
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return h;
}

// A stable random value in [0, 1) for an integer lattice cell and a salt.
fn cell_random(cell: vec3<f32>, salt: u32) -> f32 {
    let c = bitcast<vec3<u32>>(vec3<i32>(cell));
    let h = hash_u32(c.x ^ hash_u32(c.y ^ hash_u32(c.z ^ hash_u32(salt))));
    return f32(h >> 8u) / 16777216.0;
}

// Water: blocks that smoothly brighten and darken. `gp` holds 1 − modifier.
fn water_cell(light: GpuLight, cell: vec3<f32>, time: f32) -> f32 {
    let rng = cell_random(cell, 0u);
    return 1.0 - abs(sin(rng * time * 3.0 + time / 20.0)) * light.shimmer_modifier;
}

// Random: blocks that change 30 times a second.
fn random_cell(light: GpuLight, cell: vec3<f32>, time: f32) -> f32 {
    let rng = cell_random(cell, u32(floor(time * 30.0)) + 1u);
    return light.shimmer_modifier + (1.0 - light.shimmer_modifier) * rng;
}

fn shimmer_cell(light: GpuLight, cell: vec3<f32>, time: f32, water: bool) -> f32 {
    if water {
        return water_cell(light, cell, time);
    }
    return random_cell(light, cell, time);
}

// Trilinearly filtered shimmer at `world`, 0 to 1; 1 for lights without shimmer.
fn shimmer(light: GpuLight, world: vec3<f32>, time: f32) -> f32 {
    let kind = light_shimmer(light);
    if kind != LIGHT_SHIMMER_WATER && kind != LIGHT_SHIMMER_RANDOM {
        return 1.0;
    }
    let water = kind == LIGHT_SHIMMER_WATER;
    let scaled = world * light.shimmer_scale;
    let w = fract(scaled);
    let base = floor(scaled);
    let front = vec4(
        shimmer_cell(light, base, time, water),
        shimmer_cell(light, base + vec3(1.0, 0.0, 0.0), time, water),
        shimmer_cell(light, base + vec3(0.0, 1.0, 0.0), time, water),
        shimmer_cell(light, base + vec3(1.0, 1.0, 0.0), time, water),
    );
    let back = vec4(
        shimmer_cell(light, base + vec3(0.0, 0.0, 1.0), time, water),
        shimmer_cell(light, base + vec3(1.0, 0.0, 1.0), time, water),
        shimmer_cell(light, base + vec3(0.0, 1.0, 1.0), time, water),
        shimmer_cell(light, base + vec3(1.0, 1.0, 1.0), time, water),
    );
    let dz = mix(front, back, w.z);
    let dy = mix(dz.xy, dz.zw, w.y);
    return mix(dy.x, dy.y, w.x);
}
