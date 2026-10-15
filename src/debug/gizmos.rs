//! Wireframe gizmos for lights.

use bevy::math::Isometry3d;
use bevy::prelude::*;

use crate::effects::LightEffectState;
use crate::light::{DynamicLight, LightType};

/// Draws each light's radius as a sphere in its color, dimmed by its current effect intensity,
/// its shape, and the points an area light's baked shadows come from.
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
        if light.shadow_point_count() > 1 {
            let (_, rotation, _) = transform.to_scale_rotation_translation();
            for point in light.shadow_point_layout(position, rotation) {
                let at_point = Isometry3d::from_translation(point.position);
                gizmos.cross(at_point, 0.06, dimmed);
            }
        }
    }
}
