//! One texel's baked shadow from one light: a hard ray to a point-like light, soft rays over an
//! area light's shape or towards a sun's disk.

use bevy::math::Vec3;

use crate::light::LightShape;

use super::area::{AreaSampling, texel_visibility};
use super::input::BakeLight;
use super::mesh_bake::{NDOTL_TOLERANCE, Scene};
use super::mesh_builder::PreparedMesh;
use super::random;
use super::shadow_grid::level;
use super::sun::sun_visibility;
use super::texels::SurfaceTexel;

/// The texel's shadow level and the rays traced, or `None` when the light doesn't reach it or is
/// behind it. `occluded(direction, distance)` traces one ray from just above the texel.
pub fn texel_shadow(
    scene: &Scene,
    prepared: &PreparedMesh,
    light: &BakeLight,
    texel: &SurfaceTexel,
    mut occluded: impl FnMut(Vec3, f32) -> bool,
) -> Option<(u8, u32)> {
    let sampling = AreaSampling {
        samples: scene.options.area_light_samples,
        minimum: scene.options.area_light_min_samples,
    };
    // Samples are seeded per (mesh, light, texel).
    let rng = || {
        random::stream(&[
            scene.options.seed,
            prepared.mesh.id.0,
            light.id.0,
            u64::from(texel.x),
            u64::from(texel.y),
        ])
    };
    let visibility = match light.shape {
        LightShape::Point => return point_texel(light, texel, occluded),
        LightShape::Area(_) => texel_visibility(
            &light.area_shape()?,
            light.shape_reach(),
            (texel.position, texel.normal),
            sampling,
            &mut rng(),
            occluded,
        )?,
        LightShape::Sun(sun) => {
            let towards_sun = |direction| occluded(direction, f32::INFINITY);
            sun_visibility(&sun, texel.normal, sampling, &mut rng(), towards_sun)?
        }
    };
    let lit = level(visibility.fraction, light.shadow_bits_per_texel());
    Some((lit, visibility.rays))
}

/// One texel's hard shadow from a point-like light: its level (0 or 1) and rays traced, or `None`
/// when the light is out of range or behind the texel.
fn point_texel(
    light: &BakeLight,
    texel: &SurfaceTexel,
    mut occluded: impl FnMut(Vec3, f32) -> bool,
) -> Option<(u8, u32)> {
    let to_light = light.position - texel.position;
    let distance_sqr = to_light.length_squared();
    if distance_sqr > light.radius * light.radius {
        return None;
    }
    let distance = distance_sqr.sqrt();
    if distance == 0.0 {
        return Some((1, 0));
    }
    let direction = to_light / distance;
    if texel.normal.dot(direction) <= NDOTL_TOLERANCE {
        return None;
    }
    Some((u8::from(!occluded(direction, distance)), 1))
}
