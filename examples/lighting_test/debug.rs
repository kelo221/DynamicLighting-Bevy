//! Debug views: `--debug-view lighting` or `--debug-view light-count` starts in one, and the V
//! key cycles through them.

use bevy::prelude::*;
use dynlight::shared::{DebugView, DynamicLightingSettings};

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, start_view)
        .add_systems(Update, cycle_view);
}

fn start_view(mut settings: ResMut<DynamicLightingSettings>) {
    let view = match crate::argument("--debug-view").as_deref() {
        Some("lighting") => DebugView::Lighting,
        Some("light-count") => DebugView::LightCount,
        Some(other) => {
            warn!("unknown debug view {other:?}; use `lighting` or `light-count`");
            return;
        }
        None => return,
    };
    settings.debug_view = view;
}

fn cycle_view(keys: Res<ButtonInput<KeyCode>>, mut settings: ResMut<DynamicLightingSettings>) {
    if !keys.just_pressed(KeyCode::KeyV) {
        return;
    }
    let index = DebugView::ALL
        .iter()
        .position(|view| *view == settings.debug_view)
        .unwrap_or(0);
    settings.debug_view = DebugView::ALL[(index + 1) % DebugView::ALL.len()];
    info!("debug view: {:?}", settings.debug_view);
}
