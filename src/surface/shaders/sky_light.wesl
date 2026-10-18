// The sky light at a fragment: its spherical-harmonic diffuse light and, under Metallic
// shading, its prefiltered reflection weighted like the ambient environment in `shade`. A baked
// sky is shadowed by how much of it the fragment sees, which dims both.

import dynlight::shared::shaders::types::GpuLight;
import dynlight::shared::shaders::variant::FEATURE_SKY;
import dynlight::environment::shaders::sky::{sky_irradiance, sky_local};
import dynlight::surface::shaders::bindings::{
    material, dynamic_lights, sky_specular, sky_specular_sampler, FLAG_REFLECTION,
};
import dynlight::surface::shaders::brdf::fresnel_schlick_roughness;
import dynlight::surface::shaders::light_processor::{LightSum, Shading, SurfacePoint};

// Adds a sky light of which the fragment sees the share `open`, 1 for an unshadowed sky. With
// skies off (`ShaderFeatures`) it adds nothing.
fn add_sky(sum: ptr<function, LightSum>, shading: Shading, light: GpuLight, surface: SurfacePoint, open: f32) {
    if !FEATURE_SKY || light.radius_sqr < 0.0 || open <= 0.0 {
        return;
    }
    let strength = light.color * (light.intensity * open);
    let irradiance = sky_irradiance(dynamic_lights.sky, sky_local(light, surface.normal)) * strength;
    (*sum).lights += 1u;
    if !shading.specular {
        (*sum).diffuse += irradiance;
        return;
    }
    let fresnel = fresnel_schlick_roughness(max(dot(surface.normal, shading.view), 0.0), shading.f0, shading.roughness);
    (*sum).diffuse += irradiance * ((1.0 - fresnel) * (1.0 - shading.metallic));
    // A material with its own reflection cube reflects that instead.
    if (material.flags & FLAG_REFLECTION) != 0u {
        return;
    }
    let direction = sky_local(light, reflect(-shading.view, surface.normal));
    let lod = shading.roughness * f32(textureNumLevels(sky_specular) - 1u);
    let reflected = textureSampleLevel(sky_specular, sky_specular_sampler, direction, lod).rgb;
    (*sum).reflection += reflected * strength * fresnel;
}
