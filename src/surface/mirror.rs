//! CPU reference for Metallic shading's per-light terms: the BRDF of `shaders/brdf.wesl` and the
//! specular term `add_light` in `shaders/light_processor.wesl` adds per unit of light strength.

use core::f32::consts::PI;

use bevy::math::Vec3;

use crate::light::area::{self, AreaShape};
use crate::light::{ltc, mirror};
use crate::shared::GpuLight;

fn ggx_distribution(n_dot_h: f32, roughness: f32) -> f32 {
    let alpha_sqr = roughness.powi(4);
    let denominator = n_dot_h * n_dot_h * (alpha_sqr - 1.0) + 1.0;
    alpha_sqr / (PI * denominator * denominator).max(1.0e-7)
}

fn smith_visibility(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    let k = (roughness + 1.0).powi(2) / 8.0;
    let masking = |n_dot_x: f32| n_dot_x / (n_dot_x * (1.0 - k) + k);
    masking(n_dot_v) * masking(n_dot_l)
}

fn fresnel_schlick(cos_theta: f32, f0: Vec3) -> Vec3 {
    f0 + (Vec3::ONE - f0) * (1.0 - cos_theta).clamp(0.0, 1.0).powi(5)
}

/// How a surface reflects light arriving from `l` seen from `v`, all unit vectors around
/// normal `n`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflectance {
    /// Cook-Torrance specular BRDF value (per steradian).
    pub specular: Vec3,
    /// The share of light left for diffuse reflection.
    pub diffuse_weight: Vec3,
}

pub fn reflectance(
    n: Vec3,
    v: Vec3,
    l: Vec3,
    roughness: f32,
    metallic: f32,
    f0: Vec3,
) -> Reflectance {
    let h = (v + l).normalize();
    let n_dot_v = n.dot(v).max(0.0);
    let n_dot_l = n.dot(l).max(0.0);
    let fresnel = fresnel_schlick(h.dot(v).max(0.0), f0);
    let numerator = ggx_distribution(n.dot(h).max(0.0), roughness)
        * smith_visibility(n_dot_v, n_dot_l, roughness)
        * fresnel;
    Reflectance {
        specular: numerator / (4.0 * n_dot_v * n_dot_l + 1.0e-4),
        diffuse_weight: (Vec3::ONE - fresnel) * (1.0 - metallic),
    }
}

/// The surface's view of one light for Metallic shading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewer {
    pub world: Vec3,
    /// Unit surface normal.
    pub normal: Vec3,
    /// Unit vector from the surface to the camera.
    pub view: Vec3,
    /// Perceptual roughness, already clamped like the shader's.
    pub roughness: f32,
    pub metallic: f32,
    pub f0: Vec3,
}

/// The specular light of `light` per unit of its strength (color times attenuation) and
/// pattern: a rectangle's LTC integral, a tube's (or a too small rectangle's) representative
/// point, or the light's position.
pub fn specular(light: &GpuLight, at: &Viewer) -> Vec3 {
    let (mut direction, mut scale) = ((mirror::source_point(light, at.world) - at.world), 1.0);
    let shape = AreaShape::of(light);
    if let Some(shape) = &shape {
        let reflection = (-at.view).reflect(at.normal);
        let point = area::representative_point(shape, at.world, reflection);
        direction = point - at.world;
        scale = area::specular_scale(shape, at.world, point, at.roughness);
    }
    let direction = direction.normalize();
    let reflected = reflectance(
        at.normal,
        at.view,
        direction,
        at.roughness,
        at.metallic,
        at.f0,
    );
    let point = reflected.specular * (PI * at.normal.dot(direction).max(0.0) * scale);
    shape
        .and_then(|shape| {
            ltc::rectangle_specular(&shape, at.world, at.normal, at.view, at.f0, at.roughness)
        })
        .unwrap_or(point)
}

#[cfg(test)]
mod tests {
    use bevy::math::{Quat, Vec2};

    use super::*;

    fn viewer(roughness: f32) -> Viewer {
        let angle = 35.0f32.to_radians();
        Viewer {
            world: Vec3::ZERO,
            normal: Vec3::Y,
            view: Vec3::new(angle.sin(), angle.cos(), 0.0),
            roughness,
            metallic: 1.0,
            f0: Vec3::new(0.95, 0.64, 0.54),
        }
    }

    fn panel(center: Vec3, half: Vec2) -> AreaShape {
        let forward = -center.normalize();
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, forward);
        AreaShape {
            center,
            x: rotation * Vec3::X,
            y: rotation * Vec3::Y,
            forward,
            half,
            rectangle: true,
        }
    }

    #[test]
    fn small_rectangles_reflect_about_like_their_representative_point() {
        for roughness in [0.3, 0.6] {
            let at = viewer(roughness);
            let shape = panel(Vec3::new(-1.0, 1.6, 0.2), Vec2::splat(0.01));
            let ltc =
                ltc::rectangle_specular(&shape, at.world, at.normal, at.view, at.f0, roughness)
                    .expect("large enough to integrate");
            let direction = shape.center.normalize();
            let reflected = reflectance(at.normal, at.view, direction, roughness, 1.0, at.f0);
            let point = reflected.specular * (PI * direction.y);
            let ratio = ltc / point;
            println!("r {roughness}: ltc {ltc} point {point} ratio {ratio}");
            assert!(ratio.cmpgt(Vec3::splat(0.8)).all() && ratio.cmplt(Vec3::splat(1.25)).all());
        }
    }
}
