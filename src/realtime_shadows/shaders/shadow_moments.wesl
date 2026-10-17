// First half of a shadow cubemap face: the face view's depth becomes variance moments
// (d / r, (d / r)²) of the distance d from the light, blurred horizontally.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_render::view::View;
import dynlight::realtime_shadows::shaders::shadow_kernel::{ShadowFace, blur_taps, blur_weight};

@group(0) @binding(0) var depth_texture: texture_depth_2d;
// The face view sits at the light, so the view position is the light position.
@group(0) @binding(1) var<uniform> view: View;
@group(0) @binding(2) var<uniform> face: ShadowFace;

// Moments of the depth texel at `pixel`. Empty texels (reverse-Z clears to 0) and anything past
// the light's radius count as the radius, which can't shadow anything the light reaches.
fn moments_at(pixel: vec2<i32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(depth_texture));
    let texel = clamp(pixel, vec2(0), size - 1);
    let depth = textureLoad(depth_texture, texel, 0);
    if depth <= 0.0 {
        return vec2(1.0);
    }
    let uv = (vec2<f32>(texel) + 0.5) / vec2<f32>(size);
    let clip = vec4(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = view.world_from_clip * clip;
    let d = min(distance(world.xyz / world.w, view.world_position) / face.radius, 1.0);
    return vec2(d, d * d);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(depth_texture));
    // Cube faces are left-handed: face texel x shows view column (size - 1 - x).
    let pixel = vec2(size.x - 1 - i32(in.position.x), i32(in.position.y));
    var moments = moments_at(pixel);
    var total = 1.0;
    let taps = blur_taps(face.blur);
    for (var k = 1; k <= taps; k++) {
        let weight = blur_weight(k, face.blur);
        moments += (moments_at(pixel - vec2(k, 0)) + moments_at(pixel + vec2(k, 0))) * weight;
        total += 2.0 * weight;
    }
    return vec4(moments / total, 0.0, 1.0);
}
