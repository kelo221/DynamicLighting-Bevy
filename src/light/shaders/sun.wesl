// Suns: parallel light from a disk at infinity. `forward` points towards the sun; `gp1` and
// `gp2` are the cosine and sine of the disk's angular radius. Mirrors `light::sun`.

import dynlight::shared::shaders::types::GpuLight;

// The direction within the sun's disk closest to the mirror direction `reflection`: the mirror
// direction itself when it points into the disk, otherwise the disk's edge towards it.
fn sun_specular_direction(light: GpuLight, reflection: vec3<f32>) -> vec3<f32> {
    let toward = light.forward;
    let along = dot(toward, reflection);
    if along >= light.gp1 {
        return reflection;
    }
    let side = reflection - toward * along;
    let side_length = length(side);
    if side_length < 1.0e-6 {
        return toward;
    }
    return normalize(toward * light.gp1 + side * (light.gp2 / side_length));
}

// Karis's energy normalization for `sun_specular_direction`: (α / α′)² with
// α′ = saturate(α + sin θ / 3), as area lights do per axis.
fn sun_specular_scale(light: GpuLight, roughness: f32) -> f32 {
    let alpha = roughness * roughness;
    let widened = saturate(alpha + light.gp2 / 3.0);
    let scale = alpha / max(widened, 1.0e-6);
    return scale * scale;
}
