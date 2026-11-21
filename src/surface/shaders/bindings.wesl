// The `DynLitMaterial` bind group. Bindings 0–19 are the material's own; 20 and up are the
// scene-wide lighting resources every lit material shares (23 and 24 are declared by
// `dynlight::baked_lighting::shaders::baked_data`, 25, 26 and 32 by the realtime shadows, 29 by
// `dynlight::light_clusters::shaders::clusters`), except 30–31, the reflection, which is the
// sky's and the probes' cubes unless the material has one of its own.
//
// With `DYNLIGHT_EXTERNAL` (a material that is not a `DynLitMaterial`, see `surface/external.rs`)
// the material's own bindings 0–10 and the `material` uniform are not declared, and every
// scene-wide binding n sits at 100 + n, in 120–132: clear of a base material's low numbers and
// of an extension's 100–110.

import dynlight::shared::shaders::types::DynamicLights;

// Mirrors `DynLitUniform` in `surface/material.rs`.
struct DynLitUniform {
    base_color: vec4<f32>,
    emissive: vec4<f32>,
    alpha_cutoff: f32,
    flags: u32,
    metallic: f32,
    perceptual_roughness: f32,
    normal_scale: f32,
    occlusion_strength: f32,
}

const FLAG_ALPHA_MASK: u32 = 1u;
const FLAG_UNLIT: u32 = 2u;
const FLAG_BASE_COLOR_TEXTURE: u32 = 4u;
const FLAG_EMISSIVE_TEXTURE: u32 = 8u;
const FLAG_METALLIC_ROUGHNESS_TEXTURE: u32 = 16u;
const FLAG_NORMAL_MAP: u32 = 32u;
const FLAG_OCCLUSION_TEXTURE: u32 = 64u;
const FLAG_REFLECTION: u32 = 128u;
const FLAG_METALLIC_SHADING: u32 = 256u;

@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> material: DynLitUniform;
@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(1) var base_color_texture: texture_2d<f32>;
@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(2) var base_color_sampler: sampler;
@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(3) var emissive_texture: texture_2d<f32>;
@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(4) var emissive_sampler: sampler;
@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(5) var metallic_roughness_texture: texture_2d<f32>;
@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(6) var metallic_roughness_sampler: sampler;
@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(7) var normal_map_texture: texture_2d<f32>;
@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(8) var normal_map_sampler: sampler;
@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(9) var occlusion_texture: texture_2d<f32>;
@if(!DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(10) var occlusion_sampler: sampler;

@if(DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(120) var<storage, read> dynamic_lights: DynamicLights;
@else @group(constants::MATERIAL_BIND_GROUP) @binding(20) var<storage, read> dynamic_lights: DynamicLights;
@if(DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(121) var light_cookies: texture_2d_array<f32>;
@else @group(constants::MATERIAL_BIND_GROUP) @binding(21) var light_cookies: texture_2d_array<f32>;
@if(DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(122) var light_cookie_sampler: sampler;
@else @group(constants::MATERIAL_BIND_GROUP) @binding(22) var light_cookie_sampler: sampler;
// The LTC table for rectangle-light specular (`dynlight::light::shaders::ltc`).
@if(DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(127) var area_light_lut: texture_2d_array<f32>;
@else @group(constants::MATERIAL_BIND_GROUP) @binding(27) var area_light_lut: texture_2d_array<f32>;
@if(DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(128) var area_light_lut_sampler: sampler;
@else @group(constants::MATERIAL_BIND_GROUP) @binding(28) var area_light_lut_sampler: sampler;
// What the material reflects: with `FLAG_REFLECTION` its own cube in world space (cube 0),
// otherwise `dynlight::reflection_probes::REFLECTION_CUBES`: the sky's prefiltered cube in the
// sky's frame (cube 0), then the probes' cubes in world space.
@if(DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(130) var reflection_texture: texture_cube_array<f32>;
@else @group(constants::MATERIAL_BIND_GROUP) @binding(30) var reflection_texture: texture_cube_array<f32>;
@if(DYNLIGHT_EXTERNAL) @group(constants::MATERIAL_BIND_GROUP) @binding(131) var reflection_sampler: sampler;
@else @group(constants::MATERIAL_BIND_GROUP) @binding(31) var reflection_sampler: sampler;
