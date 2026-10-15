// How a `DynLitMaterial` turns gathered light into color: material textures, normal mapping,
// Diffuse or Metallic shading and the ambient term (Henry's Diffuse and Metallic fragment
// bodies, without Unity's lightmaps and reflection probes).

import dynlight::surface::shaders::bindings::{
    material, dynamic_lights, metallic_roughness_texture, metallic_roughness_sampler,
    normal_map_texture, normal_map_sampler, occlusion_texture, occlusion_sampler,
    reflection_texture, reflection_sampler, FLAG_METALLIC_ROUGHNESS_TEXTURE,
    FLAG_OCCLUSION_TEXTURE, FLAG_REFLECTION, FLAG_METALLIC_SHADING,
};
import dynlight::surface::shaders::brdf::fresnel_schlick_roughness;
import dynlight::surface::shaders::light_processor::Shading;
import dynlight::surface::shaders::lighting::{SurfaceInput, surface_light};
import dynlight::shared::shaders::types::GLOBALS_FLAG_DEBUG_LIGHT_COUNT;

// Roughness below this makes the GGX highlight a sub-pixel spike.
const MIN_ROUGHNESS: f32 = 0.045;
// The light count shown as full red; `DEBUG_LIGHT_COUNT_MAX` in `shared::settings`.
const DEBUG_LIGHT_COUNT_MAX: f32 = 16.0;

// The light count debug view: black for no lights, then blue, green, yellow and red at
// `DEBUG_LIGHT_COUNT_MAX`.
fn light_count_color(count: u32) -> vec3<f32> {
    if count == 0u {
        return vec3(0.0);
    }
    let t = saturate(f32(count - 1u) / (DEBUG_LIGHT_COUNT_MAX - 1.0)) * 3.0;
    let stops = array(vec3(0.0, 0.2, 1.0), vec3(0.0, 0.9, 0.2), vec3(1.0, 0.9, 0.0), vec3(1.0, 0.0, 0.0));
    let i = min(u32(t), 2u);
    return mix(stops[i], stops[i + 1u], t - f32(i));
}

// A tangent-space normal map applied in the mikktspace frame (bitangent = w · normal × tangent).
fn apply_normal_map(normal: vec3<f32>, tangent: vec4<f32>, uv: vec2<f32>) -> vec3<f32> {
    let texel = textureSample(normal_map_texture, normal_map_sampler, uv).rgb * 2.0 - 1.0;
    let local = vec3(texel.xy * material.normal_scale, texel.z);
    let t = normalize(tangent.xyz - normal * dot(normal, tangent.xyz));
    let b = cross(normal, t) * sign(tangent.w);
    return normalize(t * local.x + b * local.y + normal * local.z);
}

// The lit color of a fragment before emissive. `base` is the base color, `camera` the camera
// position and `uv` the texture coordinates (ignored without UV0).
fn shade(base: vec3<f32>, input: SurfaceInput, uv: vec2<f32>, camera: vec3<f32>) -> vec3<f32> {
    var metallic = material.metallic;
    var roughness = material.perceptual_roughness;
    var occlusion = 1.0;
@if(VERTEX_UVS_A) {
    if (material.flags & FLAG_METALLIC_ROUGHNESS_TEXTURE) != 0u {
        let texel = textureSample(metallic_roughness_texture, metallic_roughness_sampler, uv);
        roughness *= texel.g;
        metallic *= texel.b;
    }
    if (material.flags & FLAG_OCCLUSION_TEXTURE) != 0u {
        let ao = textureSample(occlusion_texture, occlusion_sampler, uv).r;
        occlusion = mix(1.0, ao, material.occlusion_strength);
    }
}
    roughness = clamp(roughness, MIN_ROUGHNESS, 1.0);
    let view = normalize(camera - input.world);
    let metallic_shading = (material.flags & FLAG_METALLIC_SHADING) != 0u;
    let f0 = mix(vec3(0.04), base, metallic);
    let lights = surface_light(input, Shading(view, f0, roughness, metallic, metallic_shading));
    if (dynamic_lights.globals.flags & GLOBALS_FLAG_DEBUG_LIGHT_COUNT) != 0u {
        return light_count_color(lights.lights);
    }
    let ambient = dynamic_lights.globals.ambient;
    if !metallic_shading {
        return base * (ambient + lights.diffuse) * occlusion;
    }

    let fresnel = fresnel_schlick_roughness(max(dot(input.normal, view), 0.0), f0, roughness);
    let diffuse_weight = (1.0 - fresnel) * (1.0 - metallic);
    var environment = ambient;
    if (material.flags & FLAG_REFLECTION) != 0u {
        let direction = reflect(-view, input.normal);
        let lod = roughness * f32(textureNumLevels(reflection_texture) - 1u);
        environment = textureSampleLevel(reflection_texture, reflection_sampler, direction, lod).rgb;
    }
    let lit = base * (diffuse_weight * ambient + lights.diffuse) + lights.specular;
    return lit * occlusion + environment * fresnel;
}
