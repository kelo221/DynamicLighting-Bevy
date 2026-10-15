// The volumetric fog post-process: reconstructs each pixel's world position from the depth
// prepass and blends every fog volume along the camera ray over the scene color.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_render::view::View;
import dynlight::shared::shaders::types::GpuLight;
import dynlight::fog::shaders::fog_volumes::{Fog, add_fog, compose, fog_opacity};

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@if(MULTISAMPLED)
@group(0) @binding(2) var depth_texture: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(2) var depth_texture: texture_depth_2d;
@group(0) @binding(3) var<uniform> view: View;
@group(0) @binding(4) var<storage, read> volumes: array<GpuLight>;

// Reverse-Z with an infinite far plane stores 0 for the sky; keep the ray finite.
const MIN_DEPTH: f32 = 1.0e-6;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let scene = textureSample(screen_texture, screen_sampler, in.uv);
    let depth = max(textureLoad(depth_texture, vec2<i32>(in.position.xy), 0), MIN_DEPTH);
    let clip = vec4(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, depth, 1.0);
    let world = view.world_from_clip * clip;
    let surface = world.xyz / world.w;
    let camera = view.world_position;

    var fog = Fog(vec3(0.0), 0.0);
    let count = arrayLength(&volumes);
    for (var k = 0u; k < count; k++) {
        let volume = volumes[k];
        let opacity = fog_opacity(volume, camera, surface);
        if opacity > 0.0 {
            fog = add_fog(fog, volume.color, opacity);
        }
    }
    return vec4(compose(scene.rgb, fog), scene.a);
}
