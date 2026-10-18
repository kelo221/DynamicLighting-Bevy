//! Which `ShadowMode::Realtime` lights get a shadow cubemap this frame.

use bevy::camera::primitives::Frustum;
use bevy::prelude::*;

use super::faces::select_shadow_lights;
use crate::light::{BakedInPlace, DynamicLight, LightingViewer, ShadowMode, viewer_rank};
use crate::portals::{self, RoomGraph, RoomView};
use crate::shared::{DynamicLightingSettings, RuntimeQuality};

/// The lights given a shadow cubemap this frame, nearest the viewer first: the
/// `ShadowMode::Realtime` lights some relevant camera may see the sphere of, through the rooms,
/// within the budget.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct ShadowChoice {
    pub lights: Vec<Entity>,
}

pub(super) type ShadowLight<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Has<BakedInPlace>,
);
type Viewer<'a> = (
    &'a Camera,
    &'a GlobalTransform,
    &'a Frustum,
    Has<Camera3d>,
    Has<LightingViewer>,
    Option<&'a RoomView>,
);

/// Chooses the lights into [`ShadowChoice`]. It runs without a renderer too, so the choice can
/// be tested headless.
pub(super) fn choose_shadow_lights(
    settings: Res<DynamicLightingSettings>,
    lights: Query<ShadowLight>,
    viewers: Query<Viewer>,
    rooms: Res<RoomGraph>,
    mut choice: ResMut<ShadowChoice>,
) {
    // Every camera that could be the viewer keeps the lights it sees; the viewer's distance
    // picks among them.
    let viewers: Vec<_> = viewers
        .iter()
        .filter_map(|(camera, transform, frustum, three_d, marked, view)| {
            let rank = viewer_rank(camera.is_active, camera.order, three_d, marked)?;
            Some((rank, transform, (frustum, view)))
        })
        .collect();
    let viewer = viewers
        .iter()
        .max_by_key(|(rank, ..)| *rank)
        .map(|(_, transform, _)| transform.translation());
    let budget = match settings.quality {
        RuntimeQuality::IntegratedGraphics => 0,
        _ => settings.shadow_cubemap_budget,
    };
    let seen = |position: Vec3, radius: f32| {
        portals::sees(
            &rooms,
            viewers.iter().map(|&(_, _, camera)| camera),
            position,
            radius,
        )
    };
    let candidates = lights
        .iter()
        // Suns have no position to see a cubemap from.
        .filter(|(_, light, _, visibility, _)| {
            light.shadows == ShadowMode::Realtime
                && !light.light_type.is_distant()
                && light.is_valid()
                && visibility.get()
        })
        .map(|(entity, light, transform, ..)| (entity, transform.translation(), light.radius))
        .filter(|&(_, position, radius)| seen(position, radius))
        .map(|(entity, position, _)| (entity, position))
        .collect();
    let selected = select_shadow_lights(candidates, viewer, budget as usize);
    if choice.lights != selected {
        choice.lights = selected;
    }
}
