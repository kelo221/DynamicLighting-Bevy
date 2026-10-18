// The sun's realtime shadows from its cascades: variance shadow maps along the sun's rays, the
// cascades' counterpart of `vsm.wesl`. Mirrors `dynlight::realtime_shadows::sun_mirror`.
//
// A material with the sun's realtime shadows binds the cascade array at 32 and samples it with
// the shadow cubemaps' sampler at 26 (`DynLitMaterial` does by default).

import dynlight::shared::shaders::types::{SunCascade, SunShadow, SUN_SHADOW_MOVING_ONLY};
import dynlight::realtime_shadows::shaders::vsm::{
    shadow_sampler, linstep, MIN_BLEEDING_CUT, TERMINATOR_START, TERMINATOR_END,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(32) var sun_cascades: texture_2d_array<f32>;

// Receivers are looked up this many texels off their surface along the normal.
const SUN_NORMAL_OFFSET: f32 = 1.5;
// The rim of a cascade, in units of its half side, which its blur reads past.
const SUN_CASCADE_MARGIN: f32 = 0.02;
// A cascade fades into the next over this share of its half side inside the rim.
const SUN_CASCADE_BLEND: f32 = 0.1;

// Where a receiver falls in a cascade: texture coordinates, depth, and how far out towards the
// rim it is (0 at the center, 1 at the edge).
struct CascadePoint {
    uv: vec2<f32>,
    depth: f32,
    rim: f32,
}

fn cascade_point(cascade: SunCascade, world: vec3<f32>, normal: vec3<f32>) -> CascadePoint {
    let point = vec4(world + normal * (cascade.texel_size * SUN_NORMAL_OFFSET), 1.0);
    let uv = vec2(dot(cascade.u, point), dot(cascade.v, point));
    let rim = 2.0 * max(abs(uv.x - 0.5), abs(uv.y - 0.5));
    // Past the far end lies behind everything the cascade holds.
    return CascadePoint(uv, min(dot(cascade.depth, point), 1.0), rim);
}

// The Chebyshev bound of a receiver at `depth` with filtered `moments`, past the bias, with the
// light-bleeding cut.
fn sun_visibility(moments: vec2<f32>, depth: f32, bias: f32) -> f32 {
    if depth - bias <= moments.x {
        return 1.0;
    }
    // Half floats can round E[x²] below E[x]²; a negative variance would give NaN.
    let variance = max(moments.y - moments.x * moments.x, 0.0);
    let d = moments.x - depth;
    let p = variance / max(variance + d * d, 1.0e-12);
    return linstep(MIN_BLEEDING_CUT, 1.0, p);
}

fn cascade_shadow(cascade: SunCascade, index: u32, at: CascadePoint) -> f32 {
    let moments = textureSampleLevel(sun_cascades, shadow_sampler, at.uv, index, 0.0).xy;
    return sun_visibility(moments, at.depth, cascade.depth_bias);
}

// How much of the sun reaches a surface at `world` with unit `normal`, whose cosine with the
// direction to the sun is `cosine`: 1 without cascades and outside them. The nearest cascade
// holding the point decides, fading into the next one at its rim.
//
// Receivers that are in the cascades themselves fade out with the angle, as with the shadow
// cubemaps, because the map can't resolve where a curved surface turns away from the sun.
// `static_receiver` marks baked meshes, which cascades of moving meshes only don't hold.
fn sun_shadow(shadow: SunShadow, world: vec3<f32>, normal: vec3<f32>, cosine: f32, static_receiver: bool) -> f32 {
    if shadow.count == 0u {
        return 1.0;
    }
    var fade = 1.0;
    if !static_receiver || (shadow.flags & SUN_SHADOW_MOVING_ONLY) == 0u {
        fade = smoothstep(TERMINATOR_START, TERMINATOR_END, cosine);
        if fade <= 0.0 {
            return 0.0;
        }
    }
    let count = min(shadow.count, 3u);
    let inner = 1.0 - SUN_CASCADE_MARGIN;
    for (var index = 0u; index < count; index++) {
        let cascade = shadow.cascades[index];
        let at = cascade_point(cascade, world, normal);
        if at.rim >= inner {
            continue;
        }
        let lit = cascade_shadow(cascade, index, at);
        let blend = linstep(inner - SUN_CASCADE_BLEND, inner, at.rim);
        if blend <= 0.0 {
            return fade * lit;
        }
        // Past the last cascade the sun is unshadowed; a point the next cascade misses keeps
        // this one's shadow.
        var next = 1.0;
        if index + 1u < count {
            let after = shadow.cascades[index + 1u];
            let there = cascade_point(after, world, normal);
            next = lit;
            if there.rim < inner {
                next = cascade_shadow(after, index + 1u, there);
            }
        }
        return fade * mix(lit, next, blend);
    }
    return fade;
}
