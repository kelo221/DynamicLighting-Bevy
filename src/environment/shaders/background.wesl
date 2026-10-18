// The sky behind the scene: a fullscreen triangle at the far plane (reverse Z), drawn after the
// opaque pass where no surface wrote depth. With `TONEMAP` (cameras without `Hdr`) it tonemaps
// like the surfaces in front of it.

import bevy_render::view::View;
@if(TONEMAP)
import bevy_core_pipeline::tonemapping::tone_mapping;
import dynlight::environment::shaders::sky::{sky_background_lod, sky_equirect_uv};

// Mirrors `BackgroundUniform` in `environment/render.rs`: the sky's axes in world space and its
// color times intensity.
struct Background {
    right: vec4<f32>,
    up: vec4<f32>,
    back: vec4<f32>,
    tint: vec4<f32>,
}

@group(0) @binding(0) var sky_texture: texture_2d<f32>;
@group(0) @binding(1) var sky_sampler: sampler;
@group(0) @binding(2) var<uniform> view: View;
@group(0) @binding(3) var<uniform> background: Background;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2(f32(index & 1u), f32((index >> 1u) & 1u)) * 4.0 - 1.0;
    return vec4(corner, 0.0, 1.0);
}

// The view-space point at normalized device coordinates `ndc` and reverse-Z `depth`.
fn view_point(ndc: vec2<f32>, depth: f32) -> vec3<f32> {
    let point = view.view_from_clip * vec4(ndc, depth, 1.0);
    return point.xyz / point.w;
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let viewport = view.main_pass_viewport;
    let uv = (position.xy - viewport.xy) / viewport.zw;
    let ndc = uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    // The pixel's ray through two depths short of the far plane, which an infinite reverse-Z
    // projection puts at infinity; an orthographic camera's rays don't meet at the eye.
    let ray = view_point(ndc, 0.5) - view_point(ndc, 1.0);
    let world = normalize((view.world_from_view * vec4(ray, 0.0)).xyz);
    let local = vec3(dot(world, background.right.xyz), dot(world, background.up.xyz), dot(world, background.back.xyz));
    // The direction changes smoothly across the image's seam, where uv jumps.
    let pixel_angle = max(length(dpdx(local)), length(dpdy(local)));
    let lod = sky_background_lod(pixel_angle, f32(textureDimensions(sky_texture).y));
    let radiance = textureSampleLevel(sky_texture, sky_sampler, sky_equirect_uv(local), lod).rgb;
    var color = vec4(radiance * background.tint.rgb, 1.0);
@if(TONEMAP) {
    color = tone_mapping(color, view.color_grading);
}
    return color;
}
