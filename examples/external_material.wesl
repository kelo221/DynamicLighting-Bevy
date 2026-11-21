// Fragment shader of `external_material.rs`: a material that is not a `DynLitMaterial`, lit by
// DynLight. Compiled with `DYNLIGHT_EXTERNAL`, which moves DynLight's bindings to 120–132.

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::view;
import dynlight::surface::shaders::light_processor::Shading;
import dynlight::surface::shaders::lighting::{SurfaceInput, surface_light};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping::tone_mapping;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> base_color: vec4<f32>;

const AMBIENT: vec3<f32> = vec3(0.03, 0.03, 0.04);

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    var input: SurfaceInput;
    input.world = in.world_position.xyz;
    let normal = normalize(in.world_normal);
    input.normal = select(-normal, normal, is_front);
    input.lightmap_uv = vec2(0.0);
    // No bake data: the dynamic path, lit by the realtime lights.
    input.tag = 0u;
    input.primitive = 0u;
    let view_direction = normalize(view.world_position - input.world);
    // Lambert diffuse only: `specular` is false.
    let shading = Shading(view_direction, vec3(0.04), 0.5, 0.0, false, true);
    let lights = surface_light(input, shading);
    var color = vec4(base_color.rgb * (AMBIENT + lights.diffuse), base_color.a);
@if(TONEMAP_IN_SHADER) {
    color = tone_mapping(color, view.color_grading);
}
    return color;
}
