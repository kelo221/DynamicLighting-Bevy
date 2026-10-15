// The separable Gaussian blur of the shadow cubemap faces: 3 taps, sigma 1, as in Henry's
// GaussianBlur shader (after SmoShaders by Daniel Ilett, MIT; see LICENSES/SmoShaders-MIT.txt).
// Weights are exp(-x² / 2σ²), normalized over the taps.

const BLUR_CENTER: f32 = 0.45186276;
const BLUR_SIDE: f32 = 0.27406862;
