// One light's contribution at one surface point: Henry's `LightProcessor.cginc` split into the
// steps the baked, dynamic-geometry and realtime paths share.

import dynlight::shared::shaders::types::{
    GpuLight, LIGHT_TYPE_POINT, LIGHT_TYPE_SPOT, LIGHT_TYPE_DISCOBALL, LIGHT_TYPE_RECTANGLE,
    light_type, light_has_cookie, light_has_shadow_cubemap, light_is_area,
};
import dynlight::light::shaders::light_types::{
    attenuation, cone, discoball_theta, pattern, shimmer, spot_theta, to_light_space,
};
import dynlight::light::shaders::area::{
    area_diffuse, area_facing_cosine, area_ltc_specular, area_representative_point, area_shape,
    area_specular_scale, nearest_point,
};
import dynlight::light::shaders::ltc::ltc_uv;
import dynlight::surface::shaders::bindings::{
    light_cookies, light_cookie_sampler, area_light_lut, area_light_lut_sampler,
};
import dynlight::surface::shaders::brdf::{PI, reflectance};
import dynlight::realtime_shadows::shaders::vsm::realtime_shadow;
import dynlight::shared::shaders::variant::{
    FEATURE_LIGHT_PATTERNS, FEATURE_AREA_LIGHTS, FEATURE_REALTIME_SHADOWS,
};

// An area light, when area lights are compiled in (`ShaderFeatures`); otherwise every light
// shines like a point light.
fn is_area(light: GpuLight) -> bool {
    return FEATURE_AREA_LIGHTS && light_is_area(light);
}

struct SurfacePoint {
    world: vec3<f32>,
    // Normalized, already flipped for back faces.
    normal: vec3<f32>,
    // Baked static geometry uses plain Lambert; everything else half-Lambert.
    baked: bool,
}

struct LightGeometry {
    // From the surface to where the light comes from: the light position, or the nearest point
    // of an area light's shape.
    to_light: vec3<f32>,
    distance_sqr: f32,
    // Normalized `to_light`.
    direction: vec3<f32>,
    // The light's entity position minus the surface position: the record's position, or an
    // area light's center. Shadow cubemaps are seen from there; only the lookup uses it.
    to_center: vec3<f32>,
    // Diffuse term; 0 means no direct light (bounce light may still arrive).
    ndotl: f32,
    // False for lights out of range or disabled (negative radius): skip them entirely.
    in_range: bool,
}

// Distance and diffuse term.
fn light_geometry(light: GpuLight, surface: SurfacePoint) -> LightGeometry {
    var geometry: LightGeometry;
    geometry.to_center = light.position - surface.world;
    let area = is_area(light);
    if area {
        let shape = area_shape(light);
        geometry.to_center = shape.center - surface.world;
        geometry.to_light = nearest_point(shape, surface.world) - surface.world;
    } else {
        geometry.to_light = geometry.to_center;
    }
    geometry.distance_sqr = dot(geometry.to_light, geometry.to_light);
    geometry.ndotl = 0.0;
    geometry.in_range = geometry.distance_sqr <= light.radius_sqr;
    if !geometry.in_range {
        return geometry;
    }
    if area {
        geometry.direction = geometry.to_light * inverseSqrt(max(geometry.distance_sqr, 1.0e-20));
        geometry.ndotl = area_diffuse(light, surface.world, surface.normal, surface.baked);
        return geometry;
    }
    geometry.direction = normalize(geometry.to_light);
    let ndotl = dot(surface.normal, geometry.direction);
    if surface.baked {
        geometry.ndotl = max(ndotl, 0.0);
    } else {
        let half_lambert = 0.5 + ndotl * 0.5;
        geometry.ndotl = half_lambert * half_lambert;
    }
    return geometry;
}

// The spot cookie at `direction`; `theta` is the spot cone cosine.
fn cookie(light: GpuLight, direction: vec3<f32>, theta: f32) -> f32 {
    let local = to_light_space(direction, light.forward, light.up);
    // Henry's projection, with v flipped for top-left texture origins.
    let offset = light.gp3 * local.xy / theta;
    let uv = vec2(0.5 - offset.x, 0.5 + offset.y);
    return textureSampleLevel(light_cookies, light_cookie_sampler, uv, i32(light.cookie_index), 0.0).r;
}

// The light-type cone or pattern, cookie, realtime shadow and shimmer at the surface, 0 to 1.
fn light_mask(light: GpuLight, surface: SurfacePoint, geometry: LightGeometry, time: f32) -> f32 {
    var map = 1.0;
    let kind = light_type(light);
    if FEATURE_LIGHT_PATTERNS {
        if kind == LIGHT_TYPE_SPOT {
            let theta = spot_theta(light, geometry.direction);
            if theta <= light.gp2 {
                return 0.0;
            }
            map = cone(theta, light.gp1, light.gp2);
            if light_has_cookie(light) {
                map *= cookie(light, geometry.direction, theta);
            }
        } else if kind == LIGHT_TYPE_DISCOBALL {
            let theta = discoball_theta(light, geometry.direction);
            if theta <= light.gp2 {
                return 0.0;
            }
            map = cone(theta, light.gp1, light.gp2);
        } else if kind != LIGHT_TYPE_POINT && !light_is_area(light) {
            map = pattern(light, geometry.to_light);
        }
    }
    if FEATURE_REALTIME_SHADOWS && light_has_shadow_cubemap(light) {
        map *= realtime_shadow(light, geometry.to_center, facing_cosine(light, surface, geometry));
    }
    return map * shimmer(light, surface.world, time);
}

// The cosine between the normal and the direction to the part of the light the surface faces
// most: the light itself, or the point of an area light's shape.
fn facing_cosine(light: GpuLight, surface: SurfacePoint, geometry: LightGeometry) -> f32 {
    if is_area(light) {
        return area_facing_cosine(light, surface.world, surface.normal);
    }
    return dot(surface.normal, geometry.direction);
}

// How much of a light's baked bounce light the cone lets through. Bounce light spreads wider
// than the direct cone and fades with the cone size (Henry's `calculate_*_bounce`).
fn bounce_mask(light: GpuLight, geometry: LightGeometry) -> f32 {
    let kind = light_type(light);
    if !FEATURE_LIGHT_PATTERNS {
        return 1.0;
    }
    if kind == LIGHT_TYPE_SPOT {
        let theta = spot_theta(light, geometry.direction);
        let size = min(1.0, 1.0 - light.gp2);
        return saturate((theta + 1.0) / max(light.gp1 + 1.0, 1.0e-6)) * size;
    }
    if kind == LIGHT_TYPE_DISCOBALL {
        let theta = discoball_theta(light, geometry.direction);
        let size = min(1.0, (1.0 - light.gp2) * 10.0);
        return cone(theta, light.gp1, 0.75) * size;
    }
    return 1.0;
}

// How the surface reflects light, from the material.
struct Shading {
    // Unit vector from the surface to the camera.
    view: vec3<f32>,
    // Reflectance at normal incidence.
    f0: vec3<f32>,
    roughness: f32,
    metallic: f32,
    // Metallic shading: a specular term per light, and diffuse light weighted by Fresnel.
    // Otherwise Henry's Diffuse shading.
    specular: bool,
}

// Light gathered at a fragment.
struct LightSum {
    // Multiplied by the base color afterwards.
    diffuse: vec3<f32>,
    // Reflected light, final.
    specular: vec3<f32>,
    // Lights that added light or bounce light, for the light count debug view.
    lights: u32,
}

// Adds one light given its shadow and pattern term `map` and its baked bounce light.
//
// Henry's Metallic shader divides diffuse light by π and applies the intensity twice; here a
// rough non-metal under Metallic shading matches Diffuse shading instead, and the specular BRDF
// is scaled by π to match (light colors carry irradiance / π).
fn add_light(sum: ptr<function, LightSum>, shading: Shading, surface: SurfacePoint, geometry: LightGeometry, light: GpuLight, map: f32, bounce: f32) {
    let strength = light.color * attenuation(light, geometry.distance_sqr);
    if map > 0.0 || bounce > 0.0 {
        (*sum).lights += 1u;
    }
    if !shading.specular {
        (*sum).diffuse += strength * (geometry.ndotl * map) + light.bounce_color * (attenuation(light, geometry.distance_sqr) * bounce);
        return;
    }
    let bounce_light = light.bounce_color * (attenuation(light, geometry.distance_sqr) * bounce * (1.0 - shading.metallic));
    // Area lights weight diffuse light by the Fresnel term at the point of their shape the mirror
    // direction points at, and tubes also reflect from it.
    var direction = geometry.direction;
    var scale = 1.0;
    if is_area(light) {
        let reflection = reflect(-shading.view, surface.normal);
        let point = area_representative_point(light, surface.world, reflection);
        direction = normalize(point - surface.world);
        scale = area_specular_scale(light, surface.world, point, shading.roughness);
    }
    let reflected = reflectance(surface.normal, shading.view, direction, shading.roughness, shading.metallic, shading.f0);
    let lambert = max(dot(surface.normal, direction), 0.0);
    var specular = reflected.specular * (PI * lambert * scale);
    if FEATURE_AREA_LIGHTS && light_type(light) == LIGHT_TYPE_RECTANGLE {
        specular = rectangle_specular(shading, surface, light, specular);
    }
    (*sum).diffuse += strength * reflected.diffuse_weight * (geometry.ndotl * map) + bounce_light;
    (*sum).specular += strength * specular * map;
}

// A rectangle's specular term integrated over its shape, or `point` (its representative
// point's) when it is too small to integrate.
fn rectangle_specular(shading: Shading, surface: SurfacePoint, light: GpuLight, point: vec3<f32>) -> vec3<f32> {
    let uv = ltc_uv(shading.roughness, dot(surface.normal, shading.view));
    let matrix = textureSampleLevel(area_light_lut, area_light_lut_sampler, uv, 0, 0.0);
    let albedo = textureSampleLevel(area_light_lut, area_light_lut_sampler, uv, 1, 0.0);
    let ltc = area_ltc_specular(light, surface.world, surface.normal, shading.view, shading.f0, matrix, albedo);
    return select(point, ltc.rgb, ltc.w > 0.0);
}

// Adds a light without baked shadows (realtime lights, and every light on unbaked meshes
// without a light BVH).
fn add_unshadowed(sum: ptr<function, LightSum>, shading: Shading, light: GpuLight, surface: SurfacePoint, time: f32) {
    let geometry = light_geometry(light, surface);
    if geometry.ndotl == 0.0 {
        return;
    }
    let map = light_mask(light, surface, geometry, time);
    if map != 0.0 {
        add_light(sum, shading, surface, geometry, light, map, 0.0);
    }
}
