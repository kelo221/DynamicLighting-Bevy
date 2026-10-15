//! CPU reference for the fog post-process: how opaque one fog volume is along a camera ray, and
//! how volumes combine with the scene color.

use bevy::math::Vec3;

use super::volumetric::VolumetricShape;
use crate::shared::math::smoothstep01;

/// One fog volume as the fog shader sees it (see `fog::pack` for the record layout).
#[derive(Clone, Copy, Debug)]
pub struct FogVolume {
    pub shape: VolumetricShape,
    pub center: Vec3,
    /// Cone axis.
    pub axis: Vec3,
    /// Box stretch per axis.
    pub scale: Vec3,
    pub radius: f32,
    pub thickness: f32,
    /// Fog opacity times the light's effect intensity.
    pub intensity: f32,
    pub inverse_visibility: f32,
    /// The cone's cosine term from `fog::cone_term`.
    pub cone_term: f32,
}

/// Opacity of `volume` along the segment from the camera to the visible surface.
pub fn opacity(volume: &FogVolume, camera: Vec3, surface: Vec3) -> f32 {
    if volume.radius <= 0.0 || volume.intensity == 0.0 {
        return 0.0;
    }
    let ray = surface - camera;
    let depth = ray.length();
    let coverage = match volume.shape {
        VolumetricShape::Sphere => {
            let along = if depth <= 1.0e-6 {
                0.0
            } else {
                ((volume.center - camera).dot(ray) / (depth * depth)).clamp(0.0, 1.0)
            };
            let distance = (camera + ray * along).distance(volume.center);
            if distance < volume.radius + 1.0e-5 {
                (volume.radius - distance) / volume.radius
            } else {
                0.0
            }
        }
        _ if depth <= 1.0e-6 => 0.0,
        VolumetricShape::Box => {
            let extent = volume.scale * volume.radius;
            let hit = ray_box(
                camera,
                ray / depth,
                volume.center - extent,
                volume.center + extent,
                depth,
            );
            hit.map_or(0.0, |(near, far)| (far - near) / (extent * 2.0).length())
        }
        VolumetricShape::ConeZ | VolumetricShape::ConeY => {
            let hit = ray_cone(
                camera,
                ray / depth,
                volume.center,
                volume.axis.normalize_or_zero(),
                volume.cone_term,
                volume.radius,
                depth,
            );
            hit.map_or(0.0, |(near, far)| (far - near) / volume.radius)
        }
    };
    let solid = (smoothstep01(coverage) * volume.thickness).clamp(0.0, 1.0);
    solid.min(depth * volume.inverse_visibility) * volume.intensity
}

/// Blends `volumes` (color, opacity) over `scene`: screen-blended fog colors, then a blend
/// towards the fog by the most opaque volume so dense fog can fully hide the scene.
pub fn compose(scene: Vec3, volumes: &[(Vec3, f32)]) -> Vec3 {
    let mut fog = Vec3::ZERO;
    let mut most_opaque = 0.0_f32;
    for &(color, opacity) in volumes {
        fog = screen(fog, color * opacity);
        most_opaque = most_opaque.max(opacity);
    }
    screen(fog, scene).lerp(fog, most_opaque.clamp(0.0, 1.0))
}

fn screen(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::ONE - (Vec3::ONE - a) * (Vec3::ONE - b)
}

fn ray_box(origin: Vec3, direction: Vec3, low: Vec3, high: Vec3, depth: f32) -> Option<(f32, f32)> {
    let inverse = direction.recip();
    let first = (low - origin) * inverse;
    let second = (high - origin) * inverse;
    let near = first.min(second).max_element().max(0.0);
    let far = first.max(second).min_element().min(depth);
    (far >= near).then_some((near, far))
}

fn ray_cone(
    origin: Vec3,
    direction: Vec3,
    tip: Vec3,
    axis: Vec3,
    cone_term: f32,
    height: f32,
    depth: f32,
) -> Option<(f32, f32)> {
    let w = origin - tip;
    let cos_sqr = cone_term.cos() * cone_term.cos();
    let sin_sqr = 1.0 - cos_sqr;
    let d_axis = direction.dot(axis);
    let w_axis = w.dot(axis);
    let a = d_axis * d_axis - cos_sqr;
    let b = 2.0 * (d_axis * w_axis - cos_sqr * direction.dot(w));
    let c = w_axis * w_axis - cos_sqr * w.dot(w);
    let discriminant = b * b - 4.0 * a * c;
    let tan_sqr = sin_sqr / cos_sqr.max(1.0e-6);
    let inside =
        (0.0..=height).contains(&w_axis) && w.dot(w) - w_axis * w_axis <= w_axis * w_axis * tan_sqr;

    let mut hits = [0.0_f32; 3];
    let mut count = 0;
    if discriminant >= 0.0 && a.abs() > 1.0e-6 {
        let root = discriminant.sqrt();
        for t in [(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)] {
            if t >= 0.0 && (0.0..=height).contains(&(w_axis + t * d_axis)) {
                hits[count] = t;
                count += 1;
            }
        }
    }
    if d_axis.abs() > 1.0e-4 {
        let cap = (height - w_axis) / d_axis;
        let point = w + direction * cap;
        let along = w_axis + cap * d_axis;
        if cap >= 0.0 && point.dot(point) - along * along <= height * height * tan_sqr {
            hits[count] = cap;
            count += 1;
        }
    }
    if !inside && count == 0 {
        return None;
    }
    hits[..count].sort_by(f32::total_cmp);
    let near = if inside { 0.0 } else { hits[0] };
    let far = if count == 0 {
        0.0
    } else {
        hits[count - 1].min(depth)
    };
    (far >= near).then_some((near, far))
}
