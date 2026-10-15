// `DynLitMaterial` fragment shader: Henry's Diffuse and Metallic shaders on Bevy's default mesh
// vertex output.
//
// Baked data is found by triangle: `TRIANGLE_ID` is set for meshes that carry triangle IDs from
// `dynlit_vertex.wesl` (the fallback for GPUs without primitive indices), otherwise
// `PRIMITIVE_INDEX` when the GPU supports `@builtin(primitive_index)`. With neither, baked
// meshes take the dynamic-geometry path.

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::mesh_functions::get_tag;
import dynlight::surface::shaders::bindings::{
    material, dynamic_lights, base_color_texture, base_color_sampler, emissive_texture,
    emissive_sampler, FLAG_ALPHA_MASK, FLAG_UNLIT, FLAG_BASE_COLOR_TEXTURE, FLAG_EMISSIVE_TEXTURE,
    FLAG_NORMAL_MAP,
};
import dynlight::shared::shaders::types::GLOBALS_FLAG_DEBUG_LIGHTING;
import dynlight::surface::shaders::lighting::SurfaceInput;
import dynlight::surface::shaders::shade::{apply_normal_map, shade};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping::tone_mapping;

@if(PRIMITIVE_INDEX)
enable primitive_index;

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
    @if(PRIMITIVE_INDEX) @builtin(primitive_index) primitive_index: u32,
    @if(TRIANGLE_ID) @location(8) @interpolate(flat) triangle_id: u32,
) -> @location(0) vec4<f32> {
    var base = material.base_color;
    var emissive = material.emissive.rgb;
    var uv = vec2(0.0);
@if(VERTEX_UVS_A) {
    uv = in.uv;
    if (material.flags & FLAG_BASE_COLOR_TEXTURE) != 0u {
        base *= textureSample(base_color_texture, base_color_sampler, uv);
    }
    if (material.flags & FLAG_EMISSIVE_TEXTURE) != 0u {
        emissive *= textureSample(emissive_texture, emissive_sampler, uv).rgb;
    }
}
@if(VERTEX_COLORS) {
    base *= in.color;
}
    if (material.flags & FLAG_ALPHA_MASK) != 0u && base.a < material.alpha_cutoff {
        discard;
    }

    var rgb = base.rgb;
    if (dynamic_lights.globals.flags & GLOBALS_FLAG_DEBUG_LIGHTING) != 0u {
        rgb = vec3(1.0);
        emissive = vec3(0.0);
    }
    if (material.flags & FLAG_UNLIT) == 0u {
        var normal = normalize(in.world_normal);
@if(VERTEX_TANGENTS && VERTEX_UVS_A) {
        if (material.flags & FLAG_NORMAL_MAP) != 0u {
            normal = apply_normal_map(normal, in.world_tangent, uv);
        }
}
        var input: SurfaceInput;
        input.world = in.world_position.xyz;
        input.normal = select(-normal, normal, is_front);
        input.lightmap_uv = vec2(0.0);
        input.tag = 0u;
        input.primitive = 0u;
@if(VERTEX_UVS_B) {
        input.lightmap_uv = in.uv_b;
}
@elif(VERTEX_UVS_A) {
        input.lightmap_uv = in.uv;
}
@if(PRIMITIVE_INDEX) {
        input.tag = get_tag(in.instance_index);
        input.primitive = primitive_index;
}
@if(TRIANGLE_ID) {
        input.tag = get_tag(in.instance_index);
        input.primitive = triangle_id;
}
        rgb = shade(rgb, input, uv, view.world_position);
    }
    var color = vec4(rgb + emissive, base.a);

@if(TONEMAP_IN_SHADER) {
    color = tone_mapping(color, view.color_grading);
}
    return color;
}
