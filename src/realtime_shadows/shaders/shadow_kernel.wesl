// The separable Gaussian blur of the shadow cubemap faces. Henry's GaussianBlur shader (after
// SmoShaders by Daniel Ilett, MIT; see LICENSES/SmoShaders-MIT.txt) has 3 taps at sigma 1,
// which leaves the shadow map's texels visible along shadow edges. Point-like lights use 5 taps
// at sigma 1.5; area lights blur wider (`ShadowFace.blur`). Weights are exp(-x² / 2σ²),
// normalized over the taps. Mirrors `dynlight::realtime_shadows::mirror::blur_kernel`.

// What both face passes know about the face they render (`ShadowFaceUniform`).
struct ShadowFace {
    radius: f32,
    layer: u32,
    // Gaussian sigma in texels.
    blur: f32,
}

const MAX_BLUR_TAPS: i32 = 6;

// Taps on each side of the center for a blur of `sigma` texels: 2 at sigma 1.5.
fn blur_taps(sigma: f32) -> i32 {
    return clamp(i32(round(sigma * 4.0 / 3.0)), 1, MAX_BLUR_TAPS);
}

// The unnormalized weight of the taps `offset` texels from the center.
fn blur_weight(offset: i32, sigma: f32) -> f32 {
    let x = f32(offset);
    return exp(-x * x / (2.0 * sigma * sigma));
}
