// `DynLitMaterial` prepass fragment shader: the alpha-mask cutout, so depth and normal prepasses
// see the same holes as the main pass. Motion vectors and deferred output stay zero (DynLit is
// forward-only).

import bevy_pbr::prepass::io::{VertexOutput, FragmentOutput};
import dynlight::surface::shaders::bindings::{
    material, base_color_texture, base_color_sampler, FLAG_ALPHA_MASK, FLAG_BASE_COLOR_TEXTURE,
};

fn alpha_mask(in: VertexOutput) {
    var alpha = material.base_color.a;
@if(VERTEX_UVS_A) {
    if (material.flags & FLAG_BASE_COLOR_TEXTURE) != 0u {
        alpha *= textureSample(base_color_texture, base_color_sampler, in.uv).a;
    }
}
@if(VERTEX_COLORS) {
    alpha *= in.color.a;
}
// Only alpha-masked materials may discard, so the others keep early depth testing.
@if(MAY_DISCARD) {
    if (material.flags & FLAG_ALPHA_MASK) != 0u && alpha < material.alpha_cutoff {
        discard;
    }
}
}

// Bevy declares `FragmentOutput` only for prepasses with color targets; a depth-only prepass,
// such as the realtime shadow pass, runs the entry point without outputs.
@if(PREPASS_FRAGMENT)
@fragment
fn fragment(in: VertexOutput) -> FragmentOutput {
    alpha_mask(in);
    var out: FragmentOutput;
@if(NORMAL_PREPASS) {
    out.normal = vec4(normalize(in.world_normal) * 0.5 + vec3(0.5), 1.0);
}
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
    out.frag_depth = in.unclipped_depth;
}
    return out;
}
@else
@fragment
fn fragment(in: VertexOutput) {
    alpha_mask(in);
}
