// Fog volume opacity along a camera ray and how volumes combine with the scene (Henry's
// `DynamicLightingPostProcessing.shader`). Mirrors `fog::mirror`.
//
// Records reuse `GpuLight` (see `fog::pack`): `channel` is the shape, `radius_sqr` the radius
// (not squared), `gp1` the thickness, `gp2` the cone term, box x scale or shaft half width, `gp3`
// and `shimmer_scale` the box y and z scale or the shaft half height and slope, `forward` the
// cone or shaft axis, which with `up` gives the frame boxes and shafts turn with.

import dynlight::shared::shaders::types::GpuLight;

const SHAPE_SPHERE: u32 = 1u;
const SHAPE_BOX: u32 = 2u;
const SHAPE_CONE_Z: u32 = 3u;
const SHAPE_CONE_Y: u32 = 4u;
const SHAPE_FRUSTUM: u32 = 5u;

struct Span {
    near: f32,
    far: f32,
    hit: bool,
}

// Where a ray is inside a box or frustum in the volume's frame (x right, y up, z along
// `forward`): `|x| <= half.x + slope * z`, `|y| <= half.y + slope * z` and
// `along.x <= z <= along.y`, clipped to `[0, depth]` against each of the six planes.
fn ray_shaft(origin: vec3<f32>, direction: vec3<f32>, volume: GpuLight, half: vec2<f32>, slope: f32, along: vec2<f32>, depth: f32) -> Span {
    let axis = normalize(volume.forward);
    let right = cross(axis, volume.up);
    let offset = origin - volume.position;
    let o = vec3(dot(offset, right), dot(offset, volume.up), dot(offset, axis));
    let d = vec3(dot(direction, right), dot(direction, volume.up), dot(direction, axis));
    var planes = array<vec4<f32>, 6>(
        vec4(1.0, 0.0, -slope, half.x),
        vec4(-1.0, 0.0, -slope, half.x),
        vec4(0.0, 1.0, -slope, half.y),
        vec4(0.0, -1.0, -slope, half.y),
        vec4(0.0, 0.0, -1.0, -along.x),
        vec4(0.0, 0.0, 1.0, along.y),
    );
    var near = 0.0;
    var far = depth;
    for (var i = 0u; i < 6u; i++) {
        let plane = planes[i];
        let facing = dot(plane.xyz, d);
        let room = plane.w - dot(plane.xyz, o);
        if abs(facing) < 1.0e-8 {
            if room < 0.0 {
                return Span(0.0, 0.0, false);
            }
        } else if facing > 0.0 {
            far = min(far, room / facing);
        } else {
            near = max(near, room / facing);
        }
    }
    return Span(near, far, far >= near);
}

fn ray_cone(origin: vec3<f32>, direction: vec3<f32>, tip: vec3<f32>, axis: vec3<f32>, cone_term: f32, height: f32, depth: f32) -> Span {
    let w = origin - tip;
    let cos_sqr = cos(cone_term) * cos(cone_term);
    let sin_sqr = 1.0 - cos_sqr;
    let d_axis = dot(direction, axis);
    let w_axis = dot(w, axis);
    let a = d_axis * d_axis - cos_sqr;
    let b = 2.0 * (d_axis * w_axis - cos_sqr * dot(direction, w));
    let c = w_axis * w_axis - cos_sqr * dot(w, w);
    let discriminant = b * b - 4.0 * a * c;
    let tan_sqr = sin_sqr / max(cos_sqr, 1.0e-6);
    let inside = w_axis >= 0.0 && w_axis <= height && dot(w, w) - w_axis * w_axis <= w_axis * w_axis * tan_sqr;

    var low = 1.0e30;
    var high = -1.0e30;
    var count = 0u;
    if discriminant >= 0.0 && abs(a) > 1.0e-6 {
        let root = sqrt(discriminant);
        for (var i = 0u; i < 2u; i++) {
            let t = (-b + select(-root, root, i == 1u)) / (2.0 * a);
            let along = w_axis + t * d_axis;
            if t >= 0.0 && along >= 0.0 && along <= height {
                low = min(low, t);
                high = max(high, t);
                count += 1u;
            }
        }
    }
    if abs(d_axis) > 1.0e-4 {
        let cap = (height - w_axis) / d_axis;
        let point = w + direction * cap;
        let along = w_axis + cap * d_axis;
        if cap >= 0.0 && dot(point, point) - along * along <= height * height * tan_sqr {
            low = min(low, cap);
            high = max(high, cap);
            count += 1u;
        }
    }
    if !inside && count == 0u {
        return Span(0.0, 0.0, false);
    }
    let near = select(low, 0.0, inside);
    let far = select(min(high, depth), 0.0, count == 0u);
    return Span(near, far, far >= near);
}

// Opacity of one fog volume between the camera and the visible surface.
fn fog_opacity(volume: GpuLight, camera: vec3<f32>, surface: vec3<f32>) -> f32 {
    let radius = volume.radius_sqr;
    if radius <= 0.0 || volume.volumetric_intensity == 0.0 {
        return 0.0;
    }
    let ray = surface - camera;
    let depth = length(ray);
    var coverage = 0.0;
    let shape = volume.channel;
    if shape == SHAPE_SPHERE {
        var along = 0.0;
        if depth > 1.0e-6 {
            along = saturate(dot(volume.position - camera, ray) / (depth * depth));
        }
        let distance = distance(camera + ray * along, volume.position);
        if distance < radius + 1.0e-5 {
            coverage = (radius - distance) / radius;
        }
    } else if depth > 1.0e-6 {
        let direction = ray / depth;
        if shape == SHAPE_BOX {
            let extent = vec3(volume.gp2, volume.gp3, volume.shimmer_scale) * radius;
            let span = ray_shaft(camera, direction, volume, extent.xy, 0.0, vec2(-extent.z, extent.z), depth);
            if span.hit {
                coverage = (span.far - span.near) / length(extent * 2.0);
            }
        } else if shape == SHAPE_FRUSTUM {
            let half = vec2(volume.gp2, volume.gp3);
            let span = ray_shaft(camera, direction, volume, half, volume.shimmer_scale, vec2(0.0, radius), depth);
            if span.hit {
                coverage = (span.far - span.near) / radius;
            }
        } else if shape == SHAPE_CONE_Z || shape == SHAPE_CONE_Y {
            let axis = normalize(volume.forward);
            let span = ray_cone(camera, direction, volume.position, axis, volume.gp2, radius, depth);
            if span.hit {
                coverage = (span.far - span.near) / radius;
            }
        }
    }
    let solid = saturate(smoothstep(0.0, 1.0, coverage) * volume.gp1);
    return min(solid, depth * volume.volumetric_visibility) * volume.volumetric_intensity;
}

fn screen(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    return 1.0 - (1.0 - a) * (1.0 - b);
}

// Accumulated fog: screen-blended fog colors and the most opaque volume.
struct Fog {
    color: vec3<f32>,
    opacity: f32,
}

fn add_fog(fog: Fog, color: vec3<f32>, opacity: f32) -> Fog {
    return Fog(screen(fog.color, color * opacity), max(fog.opacity, opacity));
}

// The scene seen through the fog; dense fog can hide the scene completely.
fn compose(scene: vec3<f32>, fog: Fog) -> vec3<f32> {
    return mix(screen(fog.color, scene), fog.color, saturate(fog.opacity));
}
