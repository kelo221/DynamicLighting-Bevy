//! Debug drawing: light gizmos, and room and portal gizmos.
//!
//! The debug views (lighting only, light count heat map) are a setting,
//! `DynamicLightingSettings::debug_view`, that the surface shaders read from the light buffer.

mod gizmos;
mod room_gizmos;

use bevy::gizmos::GizmoPlugin;
use bevy::prelude::*;

pub use gizmos::LightGizmos;

pub struct DebugPlugin;

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LightGizmos>();
    }

    fn finish(&self, app: &mut App) {
        if app.is_plugin_added::<GizmoPlugin>() {
            app.add_systems(
                Update,
                (gizmos::draw_light_gizmos, room_gizmos::draw_room_gizmos)
                    .run_if(|settings: Res<LightGizmos>| settings.enabled),
            );
        }
    }
}
