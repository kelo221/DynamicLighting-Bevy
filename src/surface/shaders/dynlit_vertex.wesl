// `DynLitMaterial` vertex shader for meshes with a triangle-ID attribute, the fallback for GPUs
// without `@builtin(primitive_index)`. It is Bevy's mesh vertex shader for static meshes (baked
// meshes don't skin or morph) plus the triangle ID, passed on unchanged. Every other mesh uses
// Bevy's own vertex shader.

import bevy_pbr::render::forward_io::{Vertex, decompress_vertex};
import bevy_pbr::render::mesh_functions;
import bevy_pbr::render::view_transformations::position_world_to_clip;

// `forward_io::VertexOutput` plus the triangle ID; the locations must match it.
struct TriangleVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
    @if(VERTEX_UVS_B)
    @location(3) uv_b: vec2<f32>,
    @if(VERTEX_TANGENTS)
    @location(4) world_tangent: vec4<f32>,
    @if(VERTEX_COLORS)
    @location(5) color: vec4<f32>,
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    @location(6) @interpolate(flat) instance_index: u32,
    @if(VISIBILITY_RANGE_DITHER)
    @location(7) @interpolate(flat) visibility_range_dither: i32,
    // `TRIANGLE_ID_LOCATION` in `baked_lighting::triangle_ids`.
    @location(8) @interpolate(flat) triangle_id: u32,
}

@vertex
fn vertex(vertex_in: Vertex, @location(8) triangle_id: u32) -> TriangleVertexOutput {
    var out: TriangleVertexOutput;
    // `vertex_in.instance_index` rather than `vertex.instance_index` works around a wgpu DX12
    // bug, as in Bevy's mesh shader (https://github.com/gfx-rs/naga/issues/2416).
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);

@if(VERTEX_NORMALS)
    out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex_in.instance_index);

    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4(vertex.position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);

@if(VERTEX_UVS_A)
    out.uv = vertex.uv;
@if(VERTEX_UVS_B)
    out.uv_b = vertex.uv_b;
@if(VERTEX_TANGENTS)
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex.tangent, vertex_in.instance_index);
@if(VERTEX_COLORS)
    out.color = vertex.color;
@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;
@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(vertex_in.instance_index, world_from_local[3]);

    out.triangle_id = triangle_id;
    return out;
}
