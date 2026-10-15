//! Packs fog volumes into GPU light records for the fog pass.

use core::f32::consts::FRAC_PI_2;

use bevy::math::{Quat, Vec3};

use super::volumetric::{VolumetricLight, VolumetricShape};
use crate::light::DynamicLight;
use crate::shared::GpuLight;
use crate::shared::math::{inverse_lerp_clamped, lerp_clamped};

const MIN_VISIBILITY: f32 = 1.0e-5;

/// Where a light is: translation, rotation and scale from its global transform.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

/// Whether the fog volume would be visible at all.
pub fn is_active(light: &DynamicLight, fog: &VolumetricLight, effect_intensity: f32) -> bool {
    light.is_valid() && fog.radius > 0.0 && fog.intensity * effect_intensity != 0.0
}

/// The record the fog shader reads. It reuses the light layout with these fields repurposed:
///
/// - `channel`: the shape discriminant
/// - `radius_sqr`: the fog radius (not squared)
/// - `gp1`: thickness
/// - `gp2`: the cone's cosine term, or the box's x scale (`gp3` and `shimmer_scale`: y and z)
/// - `forward`: the cone axis
/// - `volumetric_visibility`: one over the visibility distance
pub fn pack(
    light: &DynamicLight,
    fog: &VolumetricLight,
    effect_intensity: f32,
    placement: Placement,
) -> GpuLight {
    let color = light.color.to_linear();
    let mut record = GpuLight {
        position: placement.position.to_array(),
        radius_sqr: fog.radius,
        channel: fog.shape as u32,
        intensity: light.intensity * effect_intensity,
        gp1: fog.thickness,
        color: [color.red, color.green, color.blue],
        up: (placement.rotation * Vec3::Y).to_array(),
        forward: (placement.rotation * Vec3::NEG_Z).to_array(),
        volumetric_intensity: fog.intensity * effect_intensity,
        volumetric_visibility: 1.0 / fog.visibility.max(MIN_VISIBILITY),
        ..GpuLight::default()
    };
    match fog.shape {
        VolumetricShape::Sphere => {}
        VolumetricShape::Box => {
            record.gp2 = placement.scale.x;
            record.gp3 = placement.scale.y;
            record.shimmer_scale = placement.scale.z;
        }
        VolumetricShape::ConeZ => record.gp2 = cone_term(light.outer_cutoff),
        VolumetricShape::ConeY => {
            record.gp2 = cone_term(light.outer_cutoff);
            record.forward = record.up;
        }
    }
    record
}

/// Henry's empirical cone width term: the cosine of 90° plus the outer cutoff, widened by up to
/// 30% as the cutoff approaches 90°.
pub fn cone_term(outer_cutoff_degrees: f32) -> f32 {
    let widen = lerp_clamped(1.0, 1.3, outer_cutoff_degrees / 90.0);
    (FRAC_PI_2 + outer_cutoff_degrees.to_radians()).cos() * widen
}

/// Radius of a sphere around the light that contains the whole fog volume, for culling.
pub fn bounding_radius(fog: &VolumetricLight, scale: Vec3, outer_cutoff_degrees: f32) -> f32 {
    match fog.shape {
        VolumetricShape::Sphere => fog.radius,
        VolumetricShape::Box => (scale.abs() * fog.radius).length(),
        VolumetricShape::ConeZ | VolumetricShape::ConeY => {
            let mut angle = outer_cutoff_degrees.clamp(0.0, 75.0);
            if outer_cutoff_degrees > 90.0 {
                angle = (1.0 - inverse_lerp_clamped(115.0, 180.0, outer_cutoff_degrees)) * 75.0;
            }
            fog.radius / angle.to_radians().cos()
        }
    }
}
