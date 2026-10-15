// The lighting loops: which lights reach a fragment and how they are shadowed
// (Henry's `DYNLIT_FRAGMENT_INTERNAL`). Ambient light is left to the caller.

import dynlight::shared::shaders::types::{
    GpuLight, LightGlobals, GLOBALS_FLAG_LIGHTING_ONLY, light_is_realtime,
    light_bounce_bits_per_pixel,
};
import dynlight::dynamic_geometry::shaders::light_bvh::{bvh_node_count, bvh_start, bvh_next};
import dynlight::dynamic_geometry::shaders::distance_cubes::cube_shadow;
import dynlight::baked_lighting::shaders::baked_data::{Receiver, receiver};
import dynlight::baked_lighting::shaders::triangles::{
    Triangle, TriangleLight, QUALITY_INTEGRATED, load_triangle, triangle_light, baked_shadow,
    baked_bounce,
};
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
// then every realtime light. Without a light BVH every light is visited unshadowed.
fn dynamic_surface_light(world: vec3<f32>, normal: vec3<f32>, shading: Shading) -> LightSum {
    let globals = dynamic_lights.globals;
    var sum = LightSum(vec3(0.0), vec3(0.0), 0u);
    let surface = SurfacePoint(world, normal, false);
    var first_unvisited = 0u;
    if bvh_node_count() > 0u {
        let shadows = (globals.flags & GLOBALS_FLAG_LIGHTING_ONLY) == 0u;
        var walk = bvh_start();
        while bvh_next(&walk, world) {
            if walk.light < globals.baked_count {
                let light = dynamic_lights.lights[walk.light];
                add_cube_shadowed(&sum, shading, light, walk.light, surface, globals.time, shadows);
            }
        }
        first_unvisited = globals.baked_count;
    }
    let count = globals.baked_count + globals.realtime_count;
    for (var k = first_unvisited; k < count; k++) {
        add_unshadowed(&sum, shading, dynamic_lights.lights[k], surface, globals.time);
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
        map = cube_shadow(slot, surface.world, light.position, geometry.distance_sqr, geometry.direction, surface.normal);
        if map == 0.0 {
            return;
        }
    }
    map *= light_mask(light, surface, geometry, time);
    add_light(sum, shading, surface, geometry, light, map, 0.0);
}

// Light on a baked mesh: the triangle's own baked lights with their shadow bits and bounce
// light, then every realtime light unshadowed. Lambert diffuse.
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
    for (var k = 0u; k < globals.realtime_count; k++) {
        let light = dynamic_lights.lights[globals.baked_count + k];
        add_unshadowed(&sum, shading, light, surface, globals.time);
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
        map = baked_shadow(triangle, entry, pixel, globals.quality);
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
