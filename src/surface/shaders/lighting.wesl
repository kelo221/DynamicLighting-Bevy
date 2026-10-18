// The lighting loops: which lights reach a fragment and how they are shadowed
// (Henry's `DYNLIT_FRAGMENT_INTERNAL`). Ambient light is left to the caller.

import dynlight::shared::shaders::types::{
    GpuLight, LightGlobals, GLOBALS_FLAG_LIGHTING_ONLY, LIGHT_TYPE_SKY, light_is_realtime,
    light_bounce_bits_per_pixel, light_shadow_bits_per_texel, light_extent, light_type,
};
import dynlight::dynamic_geometry::shaders::light_bvh::{
    BvhStack, bvh_node_count, bvh_start, bvh_next,
};
import dynlight::shared::shaders::variant::{
    QUALITY, FEATURE_REALTIME_LIGHTS, FEATURE_UNBAKED_MESHES, FEATURE_BOUNCE,
};
import dynlight::dynamic_geometry::shaders::distance_cubes::{cube_shadow, cube_area_shadow};
import dynlight::dynamic_geometry::shaders::sun_map::{sun_map_available, sun_map_shadow};
import dynlight::dynamic_geometry::shaders::sky_probes::sky_probe_openness;
import dynlight::light::shaders::area::area_shape;
import dynlight::baked_lighting::shaders::baked_data::{Receiver, receiver};
import dynlight::baked_lighting::shaders::triangles::{
    Footprint, Triangle, TriangleLight, QUALITY_INTEGRATED, load_triangle, triangle_light,
    footprint, baked_shadow, baked_bounce,
};
import dynlight::light_clusters::shaders::clusters::{RealtimeSpan, realtime_span, span_light};
import dynlight::surface::shaders::bindings::dynamic_lights;
import dynlight::surface::shaders::light_processor::{
    LightSum, Shading, SurfacePoint, add_light, add_unshadowed, bounce_mask, is_area, is_sun,
    light_geometry, light_mask,
};
import dynlight::surface::shaders::sky_light::add_sky;

// What the lighting loops need to know about a fragment.
struct SurfaceInput {
    world: vec3<f32>,
    // Normalized, already flipped for back faces.
    normal: vec3<f32>,
    // Shadow-map coordinates: UV1, or UV0 for meshes without UV1.
    lightmap_uv: vec2<f32>,
    // The mesh's `MeshTag`; 0 for unbaked meshes or when primitive indices are unavailable.
    tag: u32,
    // `@builtin(primitive_index)`.
    primitive: u32,
}

// Light from every `DynamicLight` at a fragment: the baked-triangle path for baked meshes,
// otherwise the dynamic-geometry path.
fn surface_light(input: SurfaceInput, shading: Shading) -> LightSum {
    let record = receiver(input.tag);
    if record.resolution == 0u {
        if FEATURE_UNBAKED_MESHES {
            return dynamic_surface_light(input.world, input.normal, shading);
        }
        return LightSum(vec3(0.0), vec3(0.0), vec3(0.0), 0u);
    }
    return baked_surface_light(input, record, shading);
}

// Light on a surface without bake data, with half-Lambert diffuse (Henry's dynamic-geometry
// path): the baked lights whose BVH leaves contain the point, shadowed by their distance cubes,
// the baked suns and skies, shadowed by their sun maps and the sky probes, then the realtime
// lights that can reach it. Without a light BVH every other baked light is visited unshadowed.
fn dynamic_surface_light(world: vec3<f32>, normal: vec3<f32>, shading: Shading) -> LightSum {
    let globals = dynamic_lights.globals;
    var sum = LightSum(vec3(0.0), vec3(0.0), vec3(0.0), 0u);
    let surface = SurfacePoint(world, normal, false);
    let shadows = (globals.flags & GLOBALS_FLAG_LIGHTING_ONLY) == 0u;
    let bvh_lights = globals.baked_count - globals.baked_distant;
    var unshadowed_baked = bvh_lights;
    if bvh_node_count() > 0u {
        var stack: BvhStack;
        var walk = bvh_start(&stack);
        while bvh_next(&walk, &stack, world) {
            if walk.light < bvh_lights {
                let light = dynamic_lights.lights[walk.light];
                add_cube_shadowed(&sum, shading, light, walk.light, surface, globals.time, shadows);
            }
        }
        unshadowed_baked = 0u;
    }
    // The suns come first, so the `sun`-th distant light has the `sun`-th sun map.
    for (var sun = 0u; sun < globals.baked_distant; sun++) {
        let light = dynamic_lights.lights[bvh_lights + sun];
        if light_type(light) == LIGHT_TYPE_SKY {
            var open = 1.0;
            if shadows && !light_is_realtime(light) {
                open = sky_probe_openness(world, normal);
            }
            add_sky(&sum, shading, light, surface, open);
        } else {
            add_sun_map_shadowed(&sum, shading, light, sun, surface, globals.time, shadows);
        }
    }
    var span = RealtimeSpan(0u, 0u, false);
    if FEATURE_REALTIME_LIGHTS {
        add_realtime_distant(&sum, shading, surface, globals);
        span = realtime_span(world, globals.realtime_count - globals.realtime_distant);
    }
    add_unshadowed_lights(&sum, shading, surface, globals, unshadowed_baked, span);
    return sum;
}

// The first `unshadowed_baked` lights, then the realtime lights of `span`, unshadowed. With
// `SUBGROUPS`, a subgroup whose pixels share their span visits it with broadcast values, which
// the compiler knows are the same across the subgroup, so it loads each light once for all of
// them. Both paths do the same arithmetic, so the image doesn't change.
fn add_unshadowed_lights(sum: ptr<function, LightSum>, shading: Shading, surface: SurfacePoint, globals: LightGlobals, unshadowed_baked: u32, span: RealtimeSpan) {
@if(SUBGROUPS) {
    let words = vec3(span.first, span.count, u32(span.clustered));
    let lead = subgroupBroadcastFirst(words);
    if subgroupAll(all(words == lead)) {
        let lead_span = RealtimeSpan(lead.x, lead.y, lead.z != 0u);
        visit_unshadowed_lights(sum, shading, surface, globals, unshadowed_baked, lead_span);
        return;
    }
}
    visit_unshadowed_lights(sum, shading, surface, globals, unshadowed_baked, span);
}

fn visit_unshadowed_lights(sum: ptr<function, LightSum>, shading: Shading, surface: SurfacePoint, globals: LightGlobals, unshadowed_baked: u32, span: RealtimeSpan) {
    let local_base = globals.baked_count + globals.realtime_distant;
    let count = unshadowed_baked + span.count;
    for (var k = 0u; k < count; k++) {
        var slot = k;
        if k >= unshadowed_baked {
            let realtime = span_light(span, k - unshadowed_baked);
            if local_base + realtime >= globals.baked_count + globals.realtime_count {
                continue;
            }
            slot = local_base + realtime;
        }
        add_unshadowed(sum, shading, dynamic_lights.lights[slot], surface, globals.time);
    }
}

// The realtime distant lights (suns and skies), which reach every fragment and are in no grid
// cell.
fn add_realtime_distant(sum: ptr<function, LightSum>, shading: Shading, surface: SurfacePoint, globals: LightGlobals) {
    for (var i = 0u; i < globals.realtime_distant; i++) {
        let light = dynamic_lights.lights[globals.baked_count + i];
        if light_type(light) == LIGHT_TYPE_SKY {
            add_sky(sum, shading, light, surface, 1.0);
        } else {
            add_unshadowed(sum, shading, light, surface, globals.time);
        }
    }
}

// A baked light on dynamic geometry, shadowed by its distance cube when `shadows` is set.
fn add_cube_shadowed(sum: ptr<function, LightSum>, shading: Shading, light: GpuLight, slot: u32, surface: SurfacePoint, time: f32, shadows: bool) {
    let geometry = light_geometry(light, surface);
    if geometry.ndotl == 0.0 {
        return;
    }
    var map = 1.0;
    if shadows {
        if is_area(light) {
            let shape = area_shape(light);
            let radius = sqrt(light.radius_sqr) + light_extent(light);
            map = cube_area_shadow(slot, surface.world, surface.normal, shape.center, shape.x, shape.y, shape.half, radius);
        } else {
            let to_light = light.position - surface.world;
            let distance_sqr = dot(to_light, to_light);
            let direction = to_light * inverseSqrt(max(distance_sqr, 1.0e-20));
            map = cube_shadow(slot, surface.world, light.position, distance_sqr, direction, surface.normal);
        }
        if map == 0.0 {
            return;
        }
    }
    map *= light_mask(light, surface, geometry, time);
    add_light(sum, shading, surface, geometry, light, map, 0.0);
}

// The `sun`-th baked sun on dynamic geometry, shadowed by its sun map when `shadows` is set and
// the bake has one. With suns off it shines unshadowed, like a point light.
fn add_sun_map_shadowed(sum: ptr<function, LightSum>, shading: Shading, light: GpuLight, sun: u32, surface: SurfacePoint, time: f32, shadows: bool) {
    let geometry = light_geometry(light, surface);
    if geometry.ndotl == 0.0 {
        return;
    }
    var map = 1.0;
    if shadows && is_sun(light) && sun_map_available(sun) {
        map = sun_map_shadow(sun, surface.world);
        if map == 0.0 {
            return;
        }
    }
    map *= light_mask(light, surface, geometry, time);
    add_light(sum, shading, surface, geometry, light, map, 0.0);
}

// Light on a baked mesh: the triangle's own baked lights (baked suns and skies among them) with
// their shadow bits and bounce light, then the realtime suns and skies and the realtime lights
// that can reach it, unshadowed. Lambert diffuse.
fn baked_surface_light(input: SurfaceInput, record: Receiver, shading: Shading) -> LightSum {
    let globals = dynamic_lights.globals;
    var sum = LightSum(vec3(0.0), vec3(0.0), vec3(0.0), 0u);
    let surface = SurfacePoint(input.world, input.normal, true);
    add_triangle_lights(&sum, shading, surface, input, record, globals);
    if !FEATURE_REALTIME_LIGHTS {
        return sum;
    }
    add_realtime_distant(&sum, shading, surface, globals);
    let span = realtime_span(input.world, globals.realtime_count - globals.realtime_distant);
    add_unshadowed_lights(&sum, shading, surface, globals, 0u, span);
    return sum;
}

// The baked lights of the fragment's triangle. With `SUBGROUPS`, a subgroup whose pixels all lie
// on one triangle loads the triangle and its lights once for all of them, as in
// `add_unshadowed_lights`.
fn add_triangle_lights(sum: ptr<function, LightSum>, shading: Shading, surface: SurfacePoint, input: SurfaceInput, record: Receiver, globals: LightGlobals) {
@if(SUBGROUPS) {
    let id = vec2(input.tag, input.primitive);
    let lead = subgroupBroadcastFirst(id);
    if subgroupAll(all(id == lead)) {
        visit_triangle_lights(sum, shading, surface, input.lightmap_uv, receiver(lead.x), lead.y, globals);
        return;
    }
}
    visit_triangle_lights(sum, shading, surface, input.lightmap_uv, record, input.primitive, globals);
}

fn visit_triangle_lights(sum: ptr<function, LightSum>, shading: Shading, surface: SurfacePoint, lightmap_uv: vec2<f32>, record: Receiver, primitive: u32, globals: LightGlobals) {
    let triangle = load_triangle(record, primitive);
    let at = footprint(triangle, lightmap_uv * f32(record.resolution));
    for (var k = 0u; k < triangle.light_count; k++) {
        let entry = triangle_light(triangle, k);
        // A bake that doesn't match the light buffer must not read realtime slots.
        if entry.index < globals.baked_count {
            let light = dynamic_lights.lights[entry.index];
            add_baked(sum, shading, light, surface, triangle, entry, at, globals);
        }
    }
}

// One baked light on a baked triangle (Henry's `LightProcessor.cginc` for static geometry).
fn add_baked(
    sum: ptr<function, LightSum>,
    shading: Shading,
    light: GpuLight,
    surface: SurfacePoint,
    triangle: Triangle,
    entry: TriangleLight,
    at: Footprint,
    globals: LightGlobals,
) {
    if light_type(light) == LIGHT_TYPE_SKY {
        var open = 1.0;
        if !light_is_realtime(light) {
            open = baked_shadow(triangle, entry, at, QUALITY, light_shadow_bits_per_texel(light));
        }
        add_sky(sum, shading, light, surface, open);
        return;
    }
    let geometry = light_geometry(light, surface);
    if !geometry.in_range {
        return;
    }
    let has_bounce = FEATURE_BOUNCE && entry.bounce != 0u && QUALITY != QUALITY_INTEGRATED;
    if !has_bounce && geometry.ndotl == 0.0 {
        return;
    }
    var map = 1.0;
    var bounce = 0.0;
    if !light_is_realtime(light) {
        let shadow_bits = light_shadow_bits_per_texel(light);
        map = baked_shadow(triangle, entry, at, QUALITY, shadow_bits);
        if has_bounce {
            bounce = baked_bounce(triangle, entry, at, light_bounce_bits_per_pixel(light));
        } else if map == 0.0 {
            return;
        }
    }
    if map != 0.0 {
        map *= light_mask(light, surface, geometry, globals.time);
    }
    if bounce != 0.0 {
        bounce *= bounce_mask(light, geometry);
    }
    add_light(sum, shading, surface, geometry, light, map, bounce);
}
