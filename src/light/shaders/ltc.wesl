// Linearly transformed cosines: Heitz, Dupuy, Hill and Neubelt, "Real-Time Polygonal-Light
// Shading with Linearly Transformed Cosines" (2016), written from the paper. Mirrors
// `dynlight::light::ltc`.
//
// - The table (`AREA_LIGHT_LUT`, bound by the surface) is the authors' GGX fit: 64 × 64 texels,
//   columns by perceptual roughness and rows by sqrt(1 − n·v). Layer 0 holds the inverse
//   transform's entries (m00, m20, m02, m22), layer 1 the BRDF's directional albedo without
//   (x) and with (y) Schlick's Fresnel factor.
// - A quad seen from the surface goes into the shading frame and through the inverse transform,
//   where the lobe is a clamped cosine; its integral is Lambert's polygon formula over the part
//   above the horizon.

const LUT_SIZE: f32 = 64.0;

// Where the table is read, mapped to texel centers.
fn ltc_uv(roughness: f32, n_dot_v: f32) -> vec2<f32> {
    let scale = (LUT_SIZE - 1.0) / LUT_SIZE;
    let bias = 0.5 / LUT_SIZE;
    return vec2(roughness, sqrt(1.0 - saturate(n_dot_v))) * scale + bias;
}

// The view direction projected onto the surface, or any tangent when looking straight down the
// normal (where the fit is symmetric).
fn ltc_tangent(normal: vec3<f32>, view: vec3<f32>) -> vec3<f32> {
    let projected = view - normal * dot(normal, view);
    if dot(projected, projected) > 1.0e-12 {
        return normalize(projected);
    }
    let helper = select(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), abs(normal.x) > 0.9);
    return normalize(cross(helper, normal));
}

// The z part of the clamped cosine's edge integral over the arc between unit vectors a and b,
// with the authors' rational fit of θ / sin θ (the 1 / 2π included).
fn ltc_edge(a: vec3<f32>, b: vec3<f32>) -> f32 {
    let x = dot(a, b);
    let y = abs(x);
    let numerator = 0.8543985 + (0.4965155 + 0.0145206 * y) * y;
    let denominator = 3.417594 + (4.1616724 + y) * y;
    let v = numerator / denominator;
    let theta_over_sin = select(0.5 * inverseSqrt(max(1.0 - x * x, 1.0e-7)) - v, v, x > 0.0);
    return cross(a, b).z * theta_over_sin;
}

// The integral of the transformed cosine lobe over the quad `corners` (relative to the surface,
// in order around it) for unit `normal` and `view`: 0 to 1. `matrix` is a layer-0 texel.
fn ltc_quad_integral(corners: array<vec3<f32>, 4>, normal: vec3<f32>, view: vec3<f32>, matrix: vec4<f32>) -> f32 {
    let t1 = ltc_tangent(normal, view);
    let t2 = cross(normal, t1);
    var local: array<vec3<f32>, 4>;
    for (var i = 0u; i < 4u; i++) {
        let d = vec3(dot(corners[i], t1), dot(corners[i], t2), dot(corners[i], normal));
        local[i] = vec3(matrix.x * d.x + matrix.z * d.z, d.y, matrix.y * d.x + matrix.w * d.z);
    }
    // Sutherland-Hodgman against the horizon: a quad gains at most one vertex.
    var clipped: array<vec3<f32>, 5>;
    var count = 0u;
    for (var i = 0u; i < 4u; i++) {
        let a = local[i];
        let b = local[(i + 1u) % 4u];
        if a.z >= 0.0 {
            clipped[count] = a;
            count += 1u;
        }
        if (a.z >= 0.0) != (b.z >= 0.0) {
            clipped[count] = mix(a, b, a.z / (a.z - b.z));
            count += 1u;
        }
    }
    if count < 3u {
        return 0.0;
    }
    for (var i = 0u; i < count; i++) {
        clipped[i] = clipped[i] * inverseSqrt(max(dot(clipped[i], clipped[i]), 1.0e-20));
    }
    var sum = 0.0;
    for (var i = 0u; i < count; i++) {
        sum += ltc_edge(clipped[i], clipped[(i + 1u) % count]);
    }
    return abs(sum);
}
