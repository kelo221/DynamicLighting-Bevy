// Area lights: rectangle panels and tubes. Mirrors `dynlight::light::area`.
//
// - Range: like every light type, an area light ends within its radius of its position. The
//   record's radius is the reach past the shape (the radius minus half the shape's diagonal),
//   and attenuation uses the distance to the shape's nearest point.
// - Shadows: a baked area light is one record at the shape's center. Its baked shadows store the
//   fraction of the shape each texel sees; distance cubes and shadow cubemaps are seen from the
//   center, and the realtime shadow's terminator fades with the cosine to the point of the shape
//   the surface faces most.
// - Diffuse: the mean cosine over the shape, i.e. the polygon (Lambert) or line form factor
//   divided by the solid or plane angle the shape covers. It equals a point light's cosine as
//   the shape shrinks. A rectangle also has a Lambertian emitter's cosine at its nearest point.
// - Specular: a rectangle integrates the GGX lobe over its shape with linearly transformed cosines
//   (`dynlight::light::shaders::ltc`), divided by its solid angle like the diffuse term. Tubes,
//   and rectangles too small to integrate, use one representative point on the shape with the
//   energy normalization of Karis, "Real Shading in Unreal Engine 4" (2013), applied per axis.

import dynlight::shared::shaders::types::{GpuLight, LIGHT_TYPE_RECTANGLE, light_type};
import dynlight::light::shaders::ltc::ltc_quad_integral;

const PI: f32 = 3.141592653589793;

// Below this solid angle (or plane angle, for tubes) the shape counts as a point.
const MIN_ANGLE: f32 = 1.0e-6;

struct AreaShape {
    center: vec3<f32>,
    // Unit axes: the shape spans ±half.x along x and ±half.y along y.
    x: vec3<f32>,
    y: vec3<f32>,
    // A rectangle lights only the side this points to.
    forward: vec3<f32>,
    half: vec2<f32>,
    rectangle: bool,
}

fn area_shape(light: GpuLight) -> AreaShape {
    let x = cross(light.forward, light.up);
    let rectangle = light_type(light) == LIGHT_TYPE_RECTANGLE;
    return AreaShape(light.position, x, light.up, light.forward, vec2(light.gp1, light.gp2), rectangle);
}

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    return v * inverseSqrt(max(dot(v, v), 1.0e-20));
}

// The point of the shape nearest to `world`.
fn nearest_point(shape: AreaShape, world: vec3<f32>) -> vec3<f32> {
    let local = world - shape.center;
    let x = clamp(dot(local, shape.x), -shape.half.x, shape.half.x);
    let y = clamp(dot(local, shape.y), -shape.half.y, shape.half.y);
    return shape.center + shape.x * x + shape.y * y;
}

// The integral of the direction over the great arc between unit vectors a and b, times two.
fn edge(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let c = cross(a, b);
    let s = length(c);
    return c * (atan2(s, dot(a, b)) / max(s, 1.0e-9));
}

fn triangle_solid_angle(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> f32 {
    let triple = abs(dot(a, cross(b, c)));
    return 2.0 * atan2(triple, 1.0 + dot(a, b) + dot(b, c) + dot(c, a));
}

// The rectangle's corners relative to `world`, in order around it.
fn rectangle_corners(shape: AreaShape, world: vec3<f32>) -> array<vec3<f32>, 4> {
    let base = shape.center - world;
    let hx = shape.x * shape.half.x;
    let hy = shape.y * shape.half.y;
    return array<vec3<f32>, 4>(base - hx - hy, base + hx - hy, base + hx + hy, base - hx + hy);
}

// The solid angle of a quad given the unit directions to its corners.
fn rectangle_solid_angle(d: array<vec3<f32>, 4>) -> f32 {
    return triangle_solid_angle(d[0], d[1], d[2]) + triangle_solid_angle(d[0], d[2], d[3]);
}

// Mean cosine between `normal` and the directions to the rectangle, over its solid angle; with
// `clip`, only the part above the surface's horizon counts.
fn rectangle_cosine(shape: AreaShape, world: vec3<f32>, normal: vec3<f32>, clip: bool) -> f32 {
    let base = shape.center - world;
    var corners = rectangle_corners(shape, world);
    var d: array<vec3<f32>, 4>;
    for (var i = 0u; i < 4u; i++) {
        d[i] = safe_normalize(corners[i]);
    }
    let solid = rectangle_solid_angle(d);
    if solid < MIN_ANGLE {
        return dot(normal, safe_normalize(base));
    }
    if !clip {
        let vector = edge(d[0], d[1]) + edge(d[1], d[2]) + edge(d[2], d[3]) + edge(d[3], d[0]);
        // The winding seen from `world` decides the sign; the vector points at the shape.
        let toward = select(1.0, -1.0, dot(vector, d[0] + d[1] + d[2] + d[3]) < 0.0);
        return 0.5 * toward * dot(normal, vector) / solid;
    }
    // Sutherland-Hodgman against the horizon plane: a quad gains at most one vertex.
    var clipped: array<vec3<f32>, 5>;
    var count = 0u;
    for (var i = 0u; i < 4u; i++) {
        let a = corners[i];
        let b = corners[(i + 1u) % 4u];
        let da = dot(normal, a);
        let db = dot(normal, b);
        if da >= 0.0 {
            clipped[count] = a;
            count += 1u;
        }
        if (da >= 0.0) != (db >= 0.0) {
            clipped[count] = mix(a, b, da / (da - db));
            count += 1u;
        }
    }
    if count < 3u {
        return 0.0;
    }
    var vector = vec3(0.0);
    for (var i = 0u; i < count; i++) {
        vector += edge(safe_normalize(clipped[i]), safe_normalize(clipped[(i + 1u) % count]));
    }
    return 0.5 * abs(dot(normal, vector)) / solid;
}

// The integral of the direction over the arc from unit vector a to unit vector b.
fn arc_vector(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    return (a + b) * (length(cross(a, b)) / max(1.0 + dot(a, b), 1.0e-6));
}

// Mean cosine between `normal` and the directions to the tube, over the angle it covers; with
// `clip`, only the part above the surface's horizon counts.
fn tube_cosine(shape: AreaShape, world: vec3<f32>, normal: vec3<f32>, clip: bool) -> f32 {
    var p0 = shape.center - shape.x * shape.half.x - world;
    var p1 = shape.center + shape.x * shape.half.x - world;
    let a = safe_normalize(p0);
    let b = safe_normalize(p1);
    let angle = atan2(length(cross(a, b)), dot(a, b));
    if angle < MIN_ANGLE {
        return dot(normal, safe_normalize(shape.center - world));
    }
    if clip {
        let d0 = dot(normal, p0);
        let d1 = dot(normal, p1);
        if d0 <= 0.0 && d1 <= 0.0 {
            return 0.0;
        }
        let crossing = mix(p0, p1, d0 / (d0 - d1));
        if d0 < 0.0 {
            p0 = crossing;
        } else if d1 < 0.0 {
            p1 = crossing;
        }
    }
    return dot(normal, arc_vector(safe_normalize(p0), safe_normalize(p1))) / angle;
}

// The diffuse term of an area light at a surface: the mean cosine, clipped to the horizon with
// plain Lambert on baked surfaces and wrapped with Henry's half-Lambert elsewhere, times a
// rectangle's emission cosine.
fn area_diffuse(light: GpuLight, world: vec3<f32>, normal: vec3<f32>, baked: bool) -> f32 {
    let shape = area_shape(light);
    var emission = 1.0;
    var cosine: f32;
    if shape.rectangle {
        emission = dot(shape.forward, safe_normalize(world - nearest_point(shape, world)));
        if emission <= 0.0 {
            return 0.0;
        }
        cosine = rectangle_cosine(shape, world, normal, baked);
    } else {
        cosine = tube_cosine(shape, world, normal, baked);
    }
    if baked {
        return max(cosine, 0.0) * emission;
    }
    let half_lambert = 0.5 + 0.5 * cosine;
    return half_lambert * half_lambert * emission;
}

// The largest cosine between `normal` and a direction on the great arc from unit vector a to
// unit vector b, shorter than a half turn.
fn arc_facing(a: vec3<f32>, b: vec3<f32>, normal: vec3<f32>) -> f32 {
    var best = max(dot(normal, a), dot(normal, b));
    let c = cross(a, b);
    let s = length(c);
    if s > MIN_ANGLE {
        let axis = c / s;
        let in_plane = normal - axis * dot(normal, axis);
        if dot(cross(a, in_plane), axis) > 0.0 && dot(cross(in_plane, b), axis) > 0.0 {
            best = max(best, length(in_plane));
        }
    }
    return best;
}

// The cosine between `normal` and the direction to the point of the shape the surface faces
// most. Above 0 exactly when part of the shape is above the surface's horizon; a point light's
// cosine as the shape shrinks.
fn area_facing_cosine(light: GpuLight, world: vec3<f32>, normal: vec3<f32>) -> f32 {
    let shape = area_shape(light);
    let base = shape.center - world;
    let hx = shape.x * shape.half.x;
    if !shape.rectangle {
        return arc_facing(safe_normalize(base - hx), safe_normalize(base + hx), normal);
    }
    let denominator = dot(normal, shape.forward);
    if abs(denominator) > 1.0e-6 {
        let t = dot(base, shape.forward) / denominator;
        let hit = normal * t - base;
        if t > 0.0 && abs(dot(hit, shape.x)) <= shape.half.x && abs(dot(hit, shape.y)) <= shape.half.y {
            return 1.0;
        }
    }
    let hy = shape.y * shape.half.y;
    var d = array<vec3<f32>, 4>(base - hx - hy, base + hx - hy, base + hx + hy, base - hx + hy);
    for (var i = 0u; i < 4u; i++) {
        d[i] = safe_normalize(d[i]);
    }
    var best = -1.0;
    for (var i = 0u; i < 4u; i++) {
        best = max(best, arc_facing(d[i], d[(i + 1u) % 4u], normal));
    }
    return best;
}

// The point of the shape whose reflection a surface at `world` shows along `reflection`: where
// the ray meets a rectangle (clamped to it), or the tube point closest to the ray.
fn area_representative_point(light: GpuLight, world: vec3<f32>, reflection: vec3<f32>) -> vec3<f32> {
    let shape = area_shape(light);
    if !shape.rectangle {
        let start = shape.center - shape.x * shape.half.x;
        let l0 = start - world;
        let ld = shape.x * (2.0 * shape.half.x);
        let r_ld = dot(reflection, ld);
        let t = (dot(reflection, l0) * r_ld - dot(l0, ld)) / max(dot(ld, ld) - r_ld * r_ld, 1.0e-6);
        return start + ld * saturate(t);
    }
    var aim = world + reflection * length(shape.center - world);
    let denominator = dot(reflection, shape.forward);
    if abs(denominator) > 1.0e-4 {
        let t = dot(shape.center - world, shape.forward) / denominator;
        if t > 0.0 {
            aim = world + reflection * t;
        }
    }
    return nearest_point(shape, aim);
}

// Scales the specular term of a representative point `point` so that a larger shape widens the
// highlight without adding energy: (α / α′) per axis, α′ = saturate(α + half size / 3d).
fn area_specular_scale(light: GpuLight, world: vec3<f32>, point: vec3<f32>, roughness: f32) -> f32 {
    let shape = area_shape(light);
    if shape.rectangle && dot(shape.forward, world - point) <= 0.0 {
        return 0.0;
    }
    let alpha = roughness * roughness;
    let spread = shape.half / (3.0 * max(distance(shape.center, world), 1.0e-4));
    let scale = alpha / max(saturate(vec2(alpha) + spread), vec2(1.0e-6));
    return scale.x * select(1.0, scale.y, shape.rectangle);
}

// The specular light of a rectangle at a surface per unit of light strength, for unit `normal`
// and `view` and reflectance `f0`: π times the reflected share over the solid angle, so a small
// rectangle reflects like a point light. `matrix` and `albedo` are the LTC table's layers at the
// surface's roughness and view cosine. `w` is 0 for tubes and rectangles below `MIN_ANGLE`,
// which reflect from their representative point instead.
fn area_ltc_specular(light: GpuLight, world: vec3<f32>, normal: vec3<f32>, view: vec3<f32>, f0: vec3<f32>, matrix: vec4<f32>, albedo: vec4<f32>) -> vec4<f32> {
    let shape = area_shape(light);
    if !shape.rectangle {
        return vec4(0.0);
    }
    let corners = rectangle_corners(shape, world);
    var d: array<vec3<f32>, 4>;
    for (var i = 0u; i < 4u; i++) {
        d[i] = safe_normalize(corners[i]);
    }
    let solid = rectangle_solid_angle(d);
    if solid < MIN_ANGLE {
        return vec4(0.0);
    }
    if dot(shape.forward, world - shape.center) <= 0.0 {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    let integral = ltc_quad_integral(corners, normal, view, matrix);
    let reflected = f0 * albedo.x + (1.0 - f0) * albedo.y;
    return vec4(reflected * (PI * integral / solid), 1.0);
}
