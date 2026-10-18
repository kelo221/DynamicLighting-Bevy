// The sky behind the scene: a fullscreen triangle at the far plane (reverse Z), drawn after the
// opaque pass where no surface wrote depth.

import bevy_render::view::View;
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

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let viewport = view.main_pass_viewport;
    let uv = (position.xy - viewport.xy) / viewport.zw;
    // The near plane, because an infinite reverse-Z far plane is at infinity.
    let near = view.view_from_clip * vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 1.0, 1.0);
    let world = normalize((view.world_from_view * vec4(near.xyz / near.w, 0.0)).xyz);
    let local = vec3(dot(world, background.right.xyz), dot(world, background.up.xyz), dot(world, background.back.xyz));
    // The direction changes smoothly across the image's seam, where uv jumps.
    let pixel_angle = max(length(dpdx(local)), length(dpdy(local)));
    let lod = sky_background_lod(pixel_angle, f32(textureDimensions(sky_texture).y));
    let radiance = textureSampleLevel(sky_texture, sky_sampler, sky_equirect_uv(local), lod).rgb;
    return vec4(radiance * background.tint.rgb, 1.0);
}
