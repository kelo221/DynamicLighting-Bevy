// Second half of a shadow cubemap face: the vertical blur of the moments, written into the
// face's layer of the cube array.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import dynlight::realtime_shadows::shaders::shadow_kernel::{BLUR_CENTER, BLUR_SIDE};

@group(0) @binding(0) var moments_texture: texture_2d<f32>;

fn moments_at(pixel: vec2<i32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(moments_texture));
    return textureLoad(moments_texture, clamp(pixel, vec2(0), size - 1), 0).xy;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let up = moments_at(pixel - vec2(0, 1));
    let down = moments_at(pixel + vec2(0, 1));
    let moments = moments_at(pixel) * BLUR_CENTER + (up + down) * BLUR_SIDE;
    return vec4(moments, 0.0, 1.0);
}
