// Second half of a shadow cubemap face: the vertical blur of the moments, written into the
// face's layer of the cube array.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import dynlight::realtime_shadows::shaders::shadow_kernel::{ShadowFace, blur_taps, blur_weight};

@group(0) @binding(0) var moments_texture: texture_2d<f32>;
@group(0) @binding(1) var<uniform> face: ShadowFace;

fn moments_at(pixel: vec2<i32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(moments_texture));
    return textureLoad(moments_texture, clamp(pixel, vec2(0), size - 1), 0).xy;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    var moments = moments_at(pixel);
    var total = 1.0;
    let taps = blur_taps(face.blur);
    for (var k = 1; k <= taps; k++) {
        let weight = blur_weight(k, face.blur);
        moments += (moments_at(pixel - vec2(0, k)) + moments_at(pixel + vec2(0, k))) * weight;
        total += 2.0 * weight;
    }
    return vec4(moments / total, 0.0, 1.0);
}
