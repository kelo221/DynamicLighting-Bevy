//! Wireframe gizmos for lights.

use bevy::math::Isometry3d;
use bevy::prelude::*;

use crate::effects::LightEffectState;
use crate::light::{DynamicLight, LightType};

/// Draws each light's radius as a sphere in its color, dimmed by its current effect intensity.
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
        if matches!(light.light_type, LightType::Spot) {
            let tip = position + transform.forward() * light.radius * 0.5;
            gizmos.arrow(position, tip, full);
        }
    }
}
