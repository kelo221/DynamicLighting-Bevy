// Realtime shadows from the variance shadow cubemaps (Henry's VSM test in LightProcessor).
//
// Any material that wants realtime shadows binds the cube array at 25 and its sampler at 26 of
// its bind group (`DynLitMaterial` does by default).

import dynlight::shared::shaders::types::{GpuLight, light_has_shadow_cubemap};

@group(constants::MATERIAL_BIND_GROUP) @binding(25) var shadow_cubemaps: texture_cube_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(26) var shadow_sampler: sampler;

// Depth bias, relative to the distance.
const VSM_BIAS: f32 = 0.0075;
// The light-bleeding cut never goes below this.
const MIN_BLEEDING_CUT: f32 = 0.3;

fn linstep(low: f32, high: f32, value: f32) -> f32 {
    return saturate((value - low) / max(high - low, 1.0e-6));
}

// How much of `light` reaches a surface `to_light` away from it (light position minus surface,
// `distance_sqr` its squared length): 1 without a shadow cubemap. `ndotl` widens the
// light-bleeding cut at grazing angles.
fn realtime_shadow(light: GpuLight, to_light: vec3<f32>, distance_sqr: f32, ndotl: f32) -> f32 {
    if !light_has_shadow_cubemap(light) {
        return 1.0;
    }
    let depth = sqrt(distance_sqr / light.radius_sqr);
    let moments = textureSampleLevel(shadow_cubemaps, shadow_sampler, -to_light, light.shadow_cubemap_index, 0.0).xy;
    // Half floats can round E[x²] below E[x]²; a negative variance would give NaN.
    let variance = max(moments.y - moments.x * moments.x, 0.0);
    let d = moments.x - depth;
    let p = variance / max(variance + d * d, 1.0e-12);
    let lit = select(p, 1.0, depth - VSM_BIAS * depth <= moments.x);
    return linstep(max(max(MIN_BLEEDING_CUT, 1.0 - ndotl), depth), 1.0, lit);
}
