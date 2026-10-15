// Microfacet specular reflection for `ShadingModel::Metallic`, written from the published
// formulas:
//
// - Normal distribution: GGX / Trowbridge-Reitz (Walter et al., "Microfacet Models for
//   Refraction through Rough Surfaces", 2007), with α = roughness².
// - Masking-shadowing: Smith's separable form with the Schlick-GGX approximation and
//   k = (roughness + 1)² / 8 for analytic lights (Karis, "Real Shading in Unreal Engine 4", 2013).
// - Fresnel: Schlick's approximation (Schlick, "An Inexpensive BRDF Model for Physically-based
//   Rendering", 1994), and its roughness-aware variant for uniform environment light
//   (Lagarde, "Moving Frostbite to PBR", 2014).

const PI: f32 = 3.141592653589793;

fn ggx_distribution(n_dot_h: f32, roughness: f32) -> f32 {
    let alpha = roughness * roughness;
    let alpha_sqr = alpha * alpha;
    let denominator = n_dot_h * n_dot_h * (alpha_sqr - 1.0) + 1.0;
    return alpha_sqr / max(PI * denominator * denominator, 1.0e-7);
}

fn schlick_ggx_masking(n_dot_x: f32, k: f32) -> f32 {
    return n_dot_x / (n_dot_x * (1.0 - k) + k);
}

fn smith_visibility(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = r * r / 8.0;
    return schlick_ggx_masking(n_dot_v, k) * schlick_ggx_masking(n_dot_l, k);
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(saturate(1.0 - cos_theta), 5.0);
}

fn fresnel_schlick_roughness(cos_theta: f32, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    return f0 + (max(vec3(1.0 - roughness), f0) - f0) * pow(saturate(1.0 - cos_theta), 5.0);
}

struct Reflectance {
    // Cook-Torrance specular BRDF value (per steradian).
    specular: vec3<f32>,
    // The share of light left for diffuse reflection: (1 − F) · (1 − metallic).
    diffuse_weight: vec3<f32>,
}

// The reflectance for light arriving from `l` seen from `v`, all unit vectors around normal `n`.
fn reflectance(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, roughness: f32, metallic: f32, f0: vec3<f32>) -> Reflectance {
    let h = normalize(v + l);
    let n_dot_v = max(dot(n, v), 0.0);
    let n_dot_l = max(dot(n, l), 0.0);
    let fresnel = fresnel_schlick(max(dot(h, v), 0.0), f0);
    let numerator = ggx_distribution(max(dot(n, h), 0.0), roughness) * smith_visibility(n_dot_v, n_dot_l, roughness) * fresnel;
    let specular = numerator / (4.0 * n_dot_v * n_dot_l + 1.0e-4);
    return Reflectance(specular, (1.0 - fresnel) * (1.0 - metallic));
}
