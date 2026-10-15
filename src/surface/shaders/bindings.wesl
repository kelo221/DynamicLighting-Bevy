// The `DynLitMaterial` bind group. Bindings 0–19 are the material's own; 20 and up are the
// scene-wide lighting resources every lit material shares (23 and 24 are declared by
// `dynlight::baked_lighting::shaders::baked_data`).

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

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> material: DynLitUniform;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var base_color_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var base_color_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var emissive_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var emissive_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var metallic_roughness_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var metallic_roughness_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var normal_map_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var normal_map_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var occlusion_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var occlusion_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(11) var reflection_texture: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(12) var reflection_sampler: sampler;

@group(constants::MATERIAL_BIND_GROUP) @binding(20) var<storage, read> dynamic_lights: DynamicLights;
@group(constants::MATERIAL_BIND_GROUP) @binding(21) var light_cookies: texture_2d_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(22) var light_cookie_sampler: sampler;
