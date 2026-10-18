//! Wireframe gizmos for lights.

use bevy::math::Isometry3d;
use bevy::prelude::*;

use crate::effects::LightEffectState;
use crate::light::{DynamicLight, LightType};

/// Draws each light's radius as a sphere in its color, dimmed by its current effect intensity,
/// and its shape. Suns, which have no radius, get a ring and arrows along their direction.
#[derive(Resource, Reflect, Clone, Debug, Default, PartialEq)]
#[reflect(Resource, Default, Debug, Clone, PartialEq)]
pub struct LightGizmos {
    pub enabled: bool,
}

pub(super) fn draw_light_gizmos(
    mut gizmos: Gizmos,
    lights: Query<(&DynamicLight, &GlobalTransform, Option<&LightEffectState>)>,
) {
    for (light, transform, effect) in &lights {
        let intensity = effect
            .map_or(1.0, LightEffectState::intensity)
            .clamp(0.0, 1.0);
        let full = light.color.to_linear().with_alpha(1.0);
        let dimmed = (full * intensity).with_alpha(1.0);
        let position = transform.translation();
        let at_light = Isometry3d::from_translation(position);
        match light.light_type {
            LightType::Sun => {
                draw_sun(&mut gizmos, transform, dimmed);
                continue;
            }
            // The sky is all around; the background shows it.
            LightType::Sky => continue,
            _ => {}
        }
        gizmos.sphere(at_light, light.radius, dimmed).resolution(24);
        gizmos.cross(at_light, 0.15, full);
        let half = light.half_extents();
        let (x, y) = (transform.right() * half.x, transform.up() * half.y);
        match light.light_type {
            LightType::Spot => {
                let tip = position + transform.forward() * light.radius * 0.5;
                gizmos.arrow(position, tip, full);
            }
            LightType::Rectangle => {
                let corners = [-x - y, x - y, x + y, -x + y].map(|corner| position + corner);
                gizmos.linestrip(corners.into_iter().chain([corners[0]]), full);
                let tip = position + transform.forward() * (half.x.min(half.y) + 0.25);
                gizmos.arrow(position, tip, full);
            }
            LightType::Tube => {
                gizmos.line(position - x, position + x, full);
            }
            _ => {}
        }
    }
}

/// A ring facing the sun, with arrows along the way its light travels.
fn draw_sun(gizmos: &mut Gizmos, transform: &GlobalTransform, color: LinearRgba) {
    const RING: f32 = 0.5;
    const LENGTH: f32 = 1.5;
    let position = transform.translation();
    let rotation = transform.to_scale_rotation_translation().1;
    gizmos.circle(Isometry3d::new(position, rotation), RING, color);
    let forward = transform.forward() * LENGTH;
    gizmos.arrow(position, position + forward, color);
    for side in [
        transform.right(),
        transform.up(),
        -transform.right(),
        -transform.up(),
    ] {
        let start = position + side * RING;
        gizmos.arrow(start, start + forward * 0.6, color);
    }
}
