// The lighting loops: which lights reach a fragment and how they are shadowed
// (Henry's `DYNLIT_FRAGMENT_INTERNAL`). Ambient light is left to the caller.

import dynlight::shared::shaders::types::{
    GpuLight, LightGlobals, GLOBALS_FLAG_LIGHTING_ONLY, light_is_realtime,
    light_bounce_bits_per_pixel, light_group,
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
    LightGeometry, LightSum, Shading, SurfacePoint, add_light, add_unshadowed, bounce_mask,
    light_geometry, light_mask,
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

// The baked records of one light gathered so far: a single record, or the shadow points of an
// area light (consecutive records with the same `light_group`), which shade alike and differ
// only in their shadows. Their shadow and bounce terms add up and the light is shaded once.
struct Gathered {
    // `light_group` of the records; 0 for a single record.
    group: u32,
    // False until the first record.
    open: bool,
    light: GpuLight,
    geometry: LightGeometry,
    map: f32,
    bounce: f32,
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

// Whether `light` is another shadow point of the area light being gathered.
fn continues(gathered: Gathered, light: GpuLight) -> bool {
    let group = light_group(light);
    return gathered.open && group != 0u && group == gathered.group;
}

fn start_gathering(light: GpuLight, surface: SurfacePoint) -> Gathered {
    return Gathered(light_group(light), true, light, light_geometry(light, surface), 0.0, 0.0);
}

// Adds the gathered light with its summed shadow and bounce terms.
fn shade_gathered(sum: ptr<function, LightSum>, shading: Shading, surface: SurfacePoint, gathered: Gathered, time: f32) {
    if !gathered.open || (gathered.map == 0.0 && gathered.bounce == 0.0) {
        return;
    }
    var map = gathered.map;
    var bounce = gathered.bounce;
    if map != 0.0 {
        map *= light_mask(gathered.light, surface, gathered.geometry, time);
    }
    if bounce != 0.0 {
        bounce *= bounce_mask(gathered.light, gathered.geometry);
    }
    add_light(sum, shading, surface, gathered.geometry, gathered.light, map, bounce);
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
        var gathered: Gathered;
        var walk = bvh_start();
        // One pass more than the walk to shade the last gathered light: every function the
        // drivers inline is called once.
        loop {
            let more = bvh_next(&walk, world);
            var light: GpuLight;
            if more {
                if walk.light >= globals.baked_count {
                    continue;
                }
                light = dynamic_lights.lights[walk.light];
            }
            if !more || !continues(gathered, light) {
                shade_gathered(&sum, shading, surface, gathered, globals.time);
                if !more {
                    break;
                }
                gathered = start_gathering(light, surface);
            }
            gather_cube_shadowed(&gathered, light, walk.light, surface, shadows);
        }
        first_unvisited = globals.baked_count;
    }
    let count = globals.baked_count + globals.realtime_count;
    for (var k = first_unvisited; k < count; k++) {
        add_unshadowed(&sum, shading, dynamic_lights.lights[k], surface, globals.time);
    }
    return sum;
}

// One baked record on dynamic geometry, shadowed by its distance cube when `shadows` is set.
fn gather_cube_shadowed(gathered: ptr<function, Gathered>, light: GpuLight, slot: u32, surface: SurfacePoint, shadows: bool) {
    if (*gathered).geometry.ndotl == 0.0 {
        return;
    }
    if !shadows {
        (*gathered).map += 1.0;
        return;
    }
    // Distance cubes are baked from the record's position: an area light's shadow point.
    let to_point = light.position - surface.world;
    let point_sqr = dot(to_point, to_point);
    let point = to_point * inverseSqrt(max(point_sqr, 1.0e-20));
    (*gathered).map += cube_shadow(slot, surface.world, light.position, point_sqr, point, surface.normal);
}

// Light on a baked mesh: the triangle's own baked lights with their shadow bits and bounce
// light, then every realtime light unshadowed. Lambert diffuse.
fn baked_surface_light(input: SurfaceInput, record: Receiver, shading: Shading) -> LightSum {
    let globals = dynamic_lights.globals;
    var sum = LightSum(vec3(0.0), vec3(0.0), 0u);
    let surface = SurfacePoint(input.world, input.normal, true);
    let pixel = input.lightmap_uv * f32(record.resolution);
    let triangle = load_triangle(record, input.primitive);
    var gathered: Gathered;
    // One pass more than the triangle's lights to shade the last gathered light.
    for (var k = 0u; k <= triangle.light_count; k++) {
        let more = k < triangle.light_count;
        var entry: TriangleLight;
        var light: GpuLight;
        if more {
            entry = triangle_light(triangle, k);
            // A bake that doesn't match the light buffer must not read realtime slots.
            if entry.index >= globals.baked_count {
                continue;
            }
            light = dynamic_lights.lights[entry.index];
        }
        if !more || !continues(gathered, light) {
            shade_gathered(&sum, shading, surface, gathered, globals.time);
            if !more {
                break;
            }
            gathered = start_gathering(light, surface);
        }
        gather_baked(&gathered, light, triangle, entry, pixel, globals);
    }
    for (var k = 0u; k < globals.realtime_count; k++) {
        let light = dynamic_lights.lights[globals.baked_count + k];
        add_unshadowed(&sum, shading, light, surface, globals.time);
    }
    return sum;
}

// One baked record on a baked triangle (Henry's `LightProcessor.cginc` for static geometry):
// its shadow bits and bounce light.
fn gather_baked(
    gathered: ptr<function, Gathered>,
    light: GpuLight,
    triangle: Triangle,
    entry: TriangleLight,
    pixel: vec2<f32>,
    globals: LightGlobals,
) {
    let geometry = (*gathered).geometry;
    if !geometry.in_range {
        return;
    }
    let has_bounce = entry.bounce != 0u && globals.quality != QUALITY_INTEGRATED;
    if !has_bounce && geometry.ndotl == 0.0 {
        return;
    }
    if light_is_realtime(light) {
        (*gathered).map += 1.0;
        return;
    }
    (*gathered).map += baked_shadow(triangle, entry, pixel, globals.quality);
    if has_bounce {
        let bits = light_bounce_bits_per_pixel(light);
        (*gathered).bounce += baked_bounce(triangle, entry, pixel, bits);
    }
}
