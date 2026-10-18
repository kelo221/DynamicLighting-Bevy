// The sky light: its image's frame, its spherical-harmonic diffuse light and the
// equirectangular mapping. Mirrors `environment::mirror`; textures are passed in by the caller.

import dynlight::shared::shaders::types::{GpuLight, SkyIrradiance};

const SKY_PI: f32 = 3.141592653589793;

// A world direction in the frame of the sky's image. The record holds the entity's forward
// (−Z) and up axes.
fn sky_local(light: GpuLight, direction: vec3<f32>) -> vec3<f32> {
    let right = cross(light.forward, light.up);
    return vec3(dot(direction, right), dot(direction, light.up), -dot(direction, light.forward));
}

// The sky's irradiance over π at unit normal `n` in the sky's frame, clamped at zero (the
// basis of `environment::sh::basis`).
fn sky_irradiance(sky: SkyIrradiance, n: vec3<f32>) -> vec3<f32> {
    var e = sky.sh[0].xyz * 0.282095;
    e += sky.sh[1].xyz * (0.488603 * n.y);
    e += sky.sh[2].xyz * (0.488603 * n.z);
    e += sky.sh[3].xyz * (0.488603 * n.x);
    e += sky.sh[4].xyz * (1.092548 * n.x * n.y);
    e += sky.sh[5].xyz * (1.092548 * n.y * n.z);
    e += sky.sh[6].xyz * (0.315392 * (3.0 * n.z * n.z - 1.0));
    e += sky.sh[7].xyz * (1.092548 * n.x * n.z);
    e += sky.sh[8].xyz * (0.546274 * (n.x * n.x - n.y * n.y));
    return max(e, vec3(0.0));
}

// Image coordinates of a unit direction in the sky's frame, Blender's mapping.
fn sky_equirect_uv(direction: vec3<f32>) -> vec2<f32> {
    return vec2(0.5 + atan2(direction.z, direction.x) / (2.0 * SKY_PI), acos(clamp(direction.y, -1.0, 1.0)) / SKY_PI);
}

// The mip level that matches a pixel spanning `pixel_angle` radians on an image `rows` texels
// high, whose rows each span π / rows.
fn sky_background_lod(pixel_angle: f32, rows: f32) -> f32 {
    return log2(max(pixel_angle * rows / SKY_PI, 1.0e-6));
}
