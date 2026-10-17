// The lighting loops: which lights reach a fragment and how they are shadowed
// (Henry's `DYNLIT_FRAGMENT_INTERNAL`). Ambient light is left to the caller.

import dynlight::shared::shaders::types::{
    GpuLight, LightGlobals, GLOBALS_FLAG_LIGHTING_ONLY, light_is_realtime,
    light_bounce_bits_per_pixel, light_shadow_bits_per_texel, light_is_area, light_extent,
};
import dynlight::dynamic_geometry::shaders::light_bvh::{
    BvhStack, bvh_node_count, bvh_start, bvh_next,
};
import dynlight::dynamic_geometry::shaders::distance_cubes::{cube_shadow, cube_area_shadow};
import dynlight::light::shaders::area::area_shape;
import dynlight::baked_lighting::shaders::baked_data::{Receiver, receiver};
import dynlight::baked_lighting::shaders::triangles::{
    Triangle, TriangleLight, QUALITY_INTEGRATED, load_triangle, triangle_light, baked_shadow,
    baked_bounce,
};
import dynlight::light_clusters::shaders::clusters::{realtime_span, span_light};
import dynlight::surface::shaders::bindings::dynamic_lights;
import dynlight::surface::shaders::light_processor::{
    LightSum, Shading, SurfacePoint, add_light, add_unshadowed, bounce_mask, light_geometry,
    light_mask,
};

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
        return dynamic_surface_light(input.world, input.normal, shading);
    }
    return baked_surface_light(input, record, shading);
}

// Light on a surface without bake data, with half-Lambert diffuse (Henry's dynamic-geometry
// path): the baked lights whose BVH leaves contain the point, shadowed by their distance cubes,
// then the realtime lights that can reach it. Without a light BVH every baked light is visited
// unshadowed.
fn dynamic_surface_light(world: vec3<f32>, normal: vec3<f32>, shading: Shading) -> LightSum {
    let globals = dynamic_lights.globals;
    var sum = LightSum(vec3(0.0), vec3(0.0), 0u);
    let surface = SurfacePoint(world, normal, false);
    var unshadowed_baked = globals.baked_count;
    if bvh_node_count() > 0u {
        let shadows = (globals.flags & GLOBALS_FLAG_LIGHTING_ONLY) == 0u;
        var stack: BvhStack;
        var walk = bvh_start(&stack);
        while bvh_next(&walk, &stack, world) {
            if walk.light < globals.baked_count {
                let light = dynamic_lights.lights[walk.light];
                add_cube_shadowed(&sum, shading, light, walk.light, surface, globals.time, shadows);
            }
        }
        unshadowed_baked = 0u;
    }
    let span = realtime_span(world, globals.realtime_count);
    let count = unshadowed_baked + span.count;
    for (var k = 0u; k < count; k++) {
        var slot = k;
        if k >= unshadowed_baked {
            let realtime = span_light(span, k - unshadowed_baked);
            if realtime >= globals.realtime_count {
                continue;
            }
            slot = globals.baked_count + realtime;
        }
        add_unshadowed(&sum, shading, dynamic_lights.lights[slot], surface, globals.time);
    }
    return sum;
}

// A baked light on dynamic geometry, shadowed by its distance cube when `shadows` is set.
fn add_cube_shadowed(sum: ptr<function, LightSum>, shading: Shading, light: GpuLight, slot: u32, surface: SurfacePoint, time: f32, shadows: bool) {
    let geometry = light_geometry(light, surface);
    if geometry.ndotl == 0.0 {
        return;
    }
    var map = 1.0;
    if shadows {
        if light_is_area(light) {
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

// Light on a baked mesh: the triangle's own baked lights with their shadow bits and bounce
// light, then the realtime lights that can reach it, unshadowed. Lambert diffuse.
fn baked_surface_light(input: SurfaceInput, record: Receiver, shading: Shading) -> LightSum {
    let globals = dynamic_lights.globals;
    var sum = LightSum(vec3(0.0), vec3(0.0), 0u);
    let surface = SurfacePoint(input.world, input.normal, true);
    let pixel = input.lightmap_uv * f32(record.resolution);
    let triangle = load_triangle(record, input.primitive);
    for (var k = 0u; k < triangle.light_count; k++) {
        let entry = triangle_light(triangle, k);
        // A bake that doesn't match the light buffer must not read realtime slots.
        if entry.index < globals.baked_count {
            let light = dynamic_lights.lights[entry.index];
            add_baked(&sum, shading, light, surface, triangle, entry, pixel, globals);
        }
    }
    let span = realtime_span(input.world, globals.realtime_count);
    for (var k = 0u; k < span.count; k++) {
        let realtime = span_light(span, k);
        if realtime < globals.realtime_count {
            let light = dynamic_lights.lights[globals.baked_count + realtime];
            add_unshadowed(&sum, shading, light, surface, globals.time);
        }
    }
    return sum;
}

// One baked light on a baked triangle (Henry's `LightProcessor.cginc` for static geometry).
fn add_baked(
    sum: ptr<function, LightSum>,
    shading: Shading,
    light: GpuLight,
    surface: SurfacePoint,
    triangle: Triangle,
    entry: TriangleLight,
    pixel: vec2<f32>,
    globals: LightGlobals,
) {
    let geometry = light_geometry(light, surface);
    if !geometry.in_range {
        return;
    }
    let has_bounce = entry.bounce != 0u && globals.quality != QUALITY_INTEGRATED;
    if !has_bounce && geometry.ndotl == 0.0 {
        return;
    }
    var map = 1.0;
    var bounce = 0.0;
    if !light_is_realtime(light) {
        let shadow_bits = light_shadow_bits_per_texel(light);
        map = baked_shadow(triangle, entry, pixel, globals.quality, shadow_bits);
        if has_bounce {
            bounce = baked_bounce(triangle, entry, pixel, light_bounce_bits_per_pixel(light));
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
