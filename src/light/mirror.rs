//! CPU reference for the light shader math, evaluated from the same [`GpuLight`] record the
//! shaders read. Tests and GPU readback checks compare against it.

use core::f32::consts::{FRAC_PI_2, PI, TAU};

use bevy::math::Vec3;

use super::area::{self, AreaShape};
use super::kinds::LightType;
use crate::shared::{ChannelFlags, GpuLight};

/// The point light comes from as seen from `world`: its position, or the nearest point of an
/// area light's shape. A distant light's comes from infinitely far away; see
/// [`direction_to_light`].
pub fn source_point(light: &GpuLight, world: Vec3) -> Vec3 {
    AreaShape::of(light).map_or(light.position.into(), |shape| shape.nearest_point(world))
}

/// A distant light ([`LightType::is_distant`]) in this record.
pub fn is_distant(light: &GpuLight) -> bool {
    LightType::from_bits(ChannelFlags::unpack(light.channel).light_type)
        .is_some_and(LightType::is_distant)
}

/// Unit vector from `world` towards where the light comes from.
pub fn direction_to_light(light: &GpuLight, world: Vec3) -> Vec3 {
    if is_distant(light) {
        return Vec3::from(light.forward).normalize_or_zero();
    }
    (source_point(light, world) - world).normalize_or_zero()
}

/// Brightness at `world` before shadows and the surface's cosine: attenuation (from the
/// nearest point of an area light) times the light-type pattern. Zero outside the radius or for
/// a disabled slot. Distant lights reach everywhere at their intensity.
pub fn radiance(light: &GpuLight, world: Vec3) -> f32 {
    if light.radius_sqr < 0.0 {
        return 0.0;
    }
    if is_distant(light) {
        return attenuation(light, 0.0);
    }
    let distance_sqr = source_point(light, world).distance_squared(world);
    if distance_sqr > light.radius_sqr {
        return 0.0;
    }
    attenuation(light, distance_sqr) * type_factor(light, world)
}

/// The diffuse term for a surface at `world` with unit `normal`: Lambert on `baked` surfaces,
/// Henry's squared half-Lambert elsewhere, and the area-light mean cosine for area lights.
pub fn diffuse(light: &GpuLight, world: Vec3, normal: Vec3, baked: bool) -> f32 {
    if let Some(shape) = AreaShape::of(light) {
        return area::diffuse(&shape, world, normal, baked);
    }
    let cosine = normal.dot(direction_to_light(light, world));
    if baked {
        cosine.max(0.0)
    } else {
        let half_lambert = 0.5 + 0.5 * cosine;
        half_lambert * half_lambert
    }
}

/// Smooth falloff to zero at the radius: `intensity * (1 - s)² / (1 + falloff * s)` with
/// `s = distance² / radius²`.
pub fn attenuation(light: &GpuLight, distance_sqr: f32) -> f32 {
    let s = (distance_sqr / light.radius_sqr).clamp(0.0, 1.0);
    light.intensity * (1.0 - s) * (1.0 - s) / (1.0 + light.falloff * s)
}

/// The light-type pattern at `world`, 0 to 1.
pub fn type_factor(light: &GpuLight, world: Vec3) -> f32 {
    let position = Vec3::from(light.position);
    let to_light = position - world;
    let direction = to_light.normalize_or_zero();
    let forward = Vec3::from(light.forward);
    let up = Vec3::from(light.up);
    let (gp1, gp2, gp3) = (light.gp1, light.gp2, light.gp3);
    let light_type = ChannelFlags::unpack(light.channel).light_type;
    match LightType::from_bits(light_type).unwrap_or_default() {
        LightType::Point | LightType::Rectangle | LightType::Tube | LightType::Sun => 1.0,
        LightType::Spot => cone(direction.dot(forward), gp1, gp2),
        LightType::Discoball => {
            let local = to_light_space(direction, forward, up);
            cone(snap_direction(local).dot(local), gp1, gp2)
        }
        LightType::Wave => 0.7 + 0.3 * ((to_light.length() - gp1) * gp2).sin(),
        LightType::Interference => {
            let local = to_light_space(to_light, forward, up);
            let angle = local.x.hypot(local.z).atan2(local.y) * gp2;
            0.5 + 0.5 * (angle - gp1).cos()
        }
        LightType::Rotor => rotor(to_light_space(to_light, forward, up), light, gp1, gp2, gp3),
        LightType::Shock => {
            let distance = gp2 * to_light.length();
            let mut brightness = 0.9 + 0.1 * ((distance * 2.0 - gp1) * TAU).sin();
            brightness *= 0.9 + 0.1 * ((distance + gp1) * TAU).cos();
            brightness * (0.9 + 0.1 * ((distance * 0.5 - gp1) * TAU).sin())
        }
        LightType::Disco => {
            let local = to_light_space(to_light, forward, up);
            let horizontal = gp2 * local.x.atan2(local.z);
            let vertical = gp2 * local.x.hypot(local.z).atan2(local.y);
            let a = 0.5 + 0.5 * (horizontal + gp1).cos();
            let b = 0.5 + 0.5 * (vertical - gp3).cos();
            let mut scale = a + b - a * b;
            let distance = 0.5 * (local.x * local.x + local.z * local.z);
            if distance < 1.0 {
                scale *= distance;
            }
            1.0 - scale
        }
    }
}

/// Smooth cone edge between the outer (`outer_cos`) and inner (`inner_cos`) cutoff cosines.
/// Equal cutoffs give a hard edge instead of a division by zero.
fn cone(theta: f32, inner_cos: f32, outer_cos: f32) -> f32 {
    let epsilon = inner_cos - outer_cos;
    if epsilon.abs() <= 1.0e-6 {
        if theta >= inner_cos { 1.0 } else { 0.0 }
    } else {
        ((theta - outer_cos) / epsilon).clamp(0.0, 1.0)
    }
}

fn rotor(local: Vec3, light: &GpuLight, phase: f32, blades: f32, center: f32) -> f32 {
    let angle = blades * local.x.atan2(local.z);
    let mut scale = 0.5 + 0.5 * (angle + phase).cos();
    let center_size = light.radius_sqr * center.abs();
    let mut distance_sqr = local.x * local.x + local.z * local.z;
    if center < 0.0 {
        if distance_sqr < center_size {
            scale *= (distance_sqr / center_size).powf(PI);
        }
    } else {
        distance_sqr /= center_size;
        if distance_sqr < 1.0 {
            scale = 1.0 - distance_sqr + scale * distance_sqr;
        }
    }
    scale.powf(FRAC_PI_2)
}

/// Expresses `value` in the light's basis `(right, up, forward)`.
///
/// Henry's shaders build `right = cross(forward, up)` in Unity's left-handed space. In Bevy's
/// right-handed space the same picture needs the opposite order, `cross(up, forward)`.
pub fn to_light_space(value: Vec3, forward: Vec3, up: Vec3) -> Vec3 {
    let right = up.cross(forward);
    Vec3::new(value.dot(right), value.dot(up), value.dot(forward))
}

/// Snaps a direction to the nearest of the 26 axis and diagonal directions.
fn snap_direction(value: Vec3) -> Vec3 {
    let unit_cube = value / value.abs().max_element();
    let threshold = (PI / 8.0).tan();
    let snap = |c: f32| if c.abs() < threshold { 0.0 } else { c.signum() };
    Vec3::new(snap(unit_cube.x), snap(unit_cube.y), snap(unit_cube.z)).normalize_or_zero()
}
