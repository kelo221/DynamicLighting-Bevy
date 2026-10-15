// Realtime shadows from the variance shadow cubemaps (Henry's VSM test in LightProcessor, with a
// smooth terminator in place of his grazing-angle light-bleeding cut).
//
// Any material that wants realtime shadows binds the cube array at 25 and its sampler at 26 of
// its bind group (`DynLitMaterial` does by default).

import dynlight::shared::shaders::types::{GpuLight, light_extent, light_has_shadow_cubemap};

@group(constants::MATERIAL_BIND_GROUP) @binding(25) var shadow_cubemaps: texture_cube_array<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(26) var shadow_sampler: sampler;

// Depth bias, relative to the distance.
const VSM_BIAS: f32 = 0.0075;
// The light-bleeding cut never goes below this.
const MIN_BLEEDING_CUT: f32 = 0.3;
// The light fades in while the cosine between the normal and the light rises from START to END.
// START may sit a little below 0 because the depth bias keeps the map from shadowing a surface
// just past its own silhouette.
const TERMINATOR_START: f32 = -0.1;
const TERMINATOR_END: f32 = 0.3;

fn linstep(low: f32, high: f32, value: f32) -> f32 {
    return saturate((value - low) / max(high - low, 1.0e-6));
}

// How much of `light` reaches a surface `to_light` away from it (light position minus surface,
// `distance_sqr` its squared length) with unit `normal`: 1 without a shadow cubemap. Area lights
// are shadowed from their position too, and their cubemaps reach their full radius.
//
// The map can't resolve where a curved surface turns away from the light; left to the map, the
// surface goes dark along a jagged line of texels. It fades out with the angle instead.
fn realtime_shadow(light: GpuLight, to_light: vec3<f32>, distance_sqr: f32, normal: vec3<f32>) -> f32 {
    if !light_has_shadow_cubemap(light) {
        return 1.0;
    }
    let distance = sqrt(distance_sqr);
    let terminator = smoothstep(TERMINATOR_START, TERMINATOR_END, dot(normal, to_light) / max(distance, 1.0e-6));
    if terminator <= 0.0 {
        return 0.0;
    }
    let depth = distance / (sqrt(light.radius_sqr) + light_extent(light));
    let moments = textureSampleLevel(shadow_cubemaps, shadow_sampler, -to_light, light.shadow_cubemap_index, 0.0).xy;
    // Half floats can round E[x²] below E[x]²; a negative variance would give NaN.
    let variance = max(moments.y - moments.x * moments.x, 0.0);
    let d = moments.x - depth;
    let p = variance / max(variance + d * d, 1.0e-12);
    let lit = select(p, 1.0, depth - VSM_BIAS * depth <= moments.x);
    return terminator * linstep(max(MIN_BLEEDING_CUT, depth), 1.0, lit);
}
