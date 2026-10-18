//! CPU reference of `shaders/sky.wesl` and of the sky's term in the lit shader
//! (`surface/shaders/sky_light.wesl`), for tests and the GPU readback check.

use core::f32::consts::PI;

use bevy::math::{Vec2, Vec3};

use super::equirect::direction_to_uv;
use super::frame::SkyFrame;
use super::prefilter::SpecularCube;
use super::sh::{SH_COUNT, evaluate};
use crate::shared::cube::direction_to_face_uv;
use crate::shared::{GpuLight, GpuSkyIrradiance};

/// `sky_local`: a world direction in the frame of the sky's image.
pub fn sky_local(light: &GpuLight, direction: Vec3) -> Vec3 {
    SkyFrame::of(light).to_local(direction)
}

/// `sky_irradiance`: the irradiance over π at unit normal `n` in the sky's frame.
pub fn sky_irradiance(sky: &GpuSkyIrradiance, n: Vec3) -> Vec3 {
    let coefficients: [Vec3; SH_COUNT] = sky.sh.map(|c| Vec3::new(c[0], c[1], c[2]));
    evaluate(&coefficients, n)
}

/// `sky_equirect_uv`.
pub fn sky_equirect_uv(direction: Vec3) -> Vec2 {
    direction_to_uv(direction)
}

/// `sky_background_lod`.
pub fn sky_background_lod(pixel_angle: f32, rows: f32) -> f32 {
    (pixel_angle * rows / PI).max(1.0e-6).log2()
}

/// The specular cube at `direction` and fractional `lod`, bilinear within a face like the GPU
/// away from face edges.
pub fn sample_specular(cube: &SpecularCube, direction: Vec3, lod: f32) -> Vec3 {
    let last = (cube.levels.len() - 1) as f32;
    let lod = lod.clamp(0.0, last);
    let level = lod.floor() as usize;
    let at = |level: usize| {
        let size = cube.level_resolution(level);
        let (face, uv) = direction_to_face_uv(direction);
        let x = (uv.x * size as f32 - 0.5).clamp(0.0, (size - 1) as f32);
        let y = (uv.y * size as f32 - 0.5).clamp(0.0, (size - 1) as f32);
        let (x0, y0) = (x.floor() as u32, y.floor() as u32);
        let (x1, y1) = ((x0 + 1).min(size - 1), (y0 + 1).min(size - 1));
        let texel =
            |x: u32, y: u32| cube.levels[level][(face * size * size + y * size + x) as usize];
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let top = texel(x0, y0).lerp(texel(x1, y0), fx);
        let bottom = texel(x0, y1).lerp(texel(x1, y1), fx);
        top.lerp(bottom, fy)
    };
    if level as f32 == lod {
        return at(level);
    }
    at(level).lerp(at(level + 1), lod - level as f32)
}

/// What a sky light adds at a surface: `(diffuse, reflection)` of the lit shader's light sum,
/// with Diffuse shading when `metallic` is `None`, otherwise Metallic shading of that metallic
/// value with reflectance `f0`. `normal` and `view` are unit vectors; `open` is the share of the
/// sky the surface sees, 1 for an unshadowed sky.
pub struct SkySurface {
    pub normal: Vec3,
    pub view: Vec3,
    pub roughness: f32,
    pub metallic: Option<f32>,
    pub f0: Vec3,
    pub open: f32,
}

pub fn sky_light(
    light: &GpuLight,
    sky: &GpuSkyIrradiance,
    cube: &SpecularCube,
    surface: &SkySurface,
) -> (Vec3, Vec3) {
    if light.radius_sqr < 0.0 || surface.open <= 0.0 {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    let strength = Vec3::from(light.color) * (light.intensity * surface.open);
    let irradiance = sky_irradiance(sky, sky_local(light, surface.normal)) * strength;
    let Some(metallic) = surface.metallic else {
        return (irradiance, Vec3::ZERO);
    };
    let cosine = surface.normal.dot(surface.view).max(0.0);
    let fresnel = fresnel_schlick_roughness(cosine, surface.f0, surface.roughness);
    let diffuse = irradiance * (Vec3::ONE - fresnel) * (1.0 - metallic);
    let reflection = (2.0 * surface.normal.dot(surface.view)) * surface.normal - surface.view;
    let lod = surface.roughness * (cube.levels.len() - 1) as f32;
    let reflected = sample_specular(cube, sky_local(light, reflection), lod);
    (diffuse, reflected * strength * fresnel)
}

/// `fresnel_schlick_roughness` in `surface/shaders/brdf.wesl`.
pub fn fresnel_schlick_roughness(cos_theta: f32, f0: Vec3, roughness: f32) -> Vec3 {
    let grazing = Vec3::splat(1.0 - roughness).max(f0);
    f0 + (grazing - f0) * (1.0 - cos_theta).clamp(0.0, 1.0).powi(5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::equirect::{Equirect, Pyramid};
    use crate::environment::prefilter::prefilter;
    use crate::environment::sh::{irradiance_over_pi, project, to_gpu};
    use crate::light::{DynamicLight, LightPose, PackContext, pack};

    #[test]
    fn a_white_sky_lights_a_white_surface_white() {
        let image = Equirect::from_fn(64, 32, |_| Vec3::ONE);
        let sky = to_gpu(&irradiance_over_pi(project(&image)));
        let cube = prefilter(&Pyramid::new(image), 4, 3, 16);
        let light = pack(
            &DynamicLight::sky(),
            LightPose::default(),
            PackContext::default(),
        );
        let surface = SkySurface {
            normal: Vec3::Y,
            view: Vec3::new(0.0, 0.6, 0.8),
            roughness: 0.5,
            metallic: None,
            f0: Vec3::splat(0.04),
            open: 1.0,
        };
        let (diffuse, specular) = sky_light(&light, &sky, &cube, &surface);
        assert!(diffuse.abs_diff_eq(Vec3::ONE, 2e-3) && specular == Vec3::ZERO);
        // Metallic shading splits the light between diffuse and reflection.
        let metal = SkySurface {
            metallic: Some(0.0),
            ..surface
        };
        let (diffuse, specular) = sky_light(&light, &sky, &cube, &metal);
        let total = diffuse + specular;
        assert!(total.abs_diff_eq(Vec3::ONE, 3e-3), "{total}");
        assert!(specular.x > 0.04 && specular.x < 0.2, "{specular}");
        // A shadowed sky dims both alike.
        let shaded = SkySurface {
            open: 0.25,
            ..metal
        };
        let (dim_diffuse, dim_specular) = sky_light(&light, &sky, &cube, &shaded);
        assert!(dim_diffuse.abs_diff_eq(diffuse * 0.25, 1e-6));
        assert!(dim_specular.abs_diff_eq(specular * 0.25, 1e-6));
    }

    #[test]
    fn background_lod_follows_the_pixel_size() {
        assert!((sky_background_lod(PI / 512.0, 512.0)).abs() < 1e-6);
        assert!((sky_background_lod(PI / 128.0, 512.0) - 2.0).abs() < 1e-5);
    }
}
