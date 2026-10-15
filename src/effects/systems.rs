use bevy::prelude::*;

use super::{advance::advance_effect, effect::LightEffect, effect::LightEffectState};
use crate::shared::{BakeId, DynamicLightingSettings};

/// Seeds new effect states from the scene seed and the light's stable id, then advances every
/// effect by one frame of virtual time.
pub(super) fn advance_light_effects(
    time: Res<Time>,
    settings: Res<DynamicLightingSettings>,
    mut lights: Query<(Entity, &LightEffect, &mut LightEffectState, Option<&BakeId>)>,
) {
    let delta = time.delta_secs();
    let now = time.elapsed_secs();
    for (entity, effect, mut state, bake_id) in &mut lights {
        if !state.is_seeded() {
            let key = bake_id.map_or(entity.to_bits(), |id| id.0);
            state.seed(settings.seed, key);
        }
        advance_effect(effect, &mut state, delta, now);
    }
}
