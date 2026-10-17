use bevy::prelude::*;

use super::advance::advance_effect;
use super::effect::{LightEffect, LightEffectState};
use super::rng::light_key;
use crate::shared::{BakeId, DynamicLightingSettings, NamePaths};

type EffectItem<'a> = (
    Entity,
    &'a LightEffect,
    &'a mut LightEffectState,
    Option<&'a BakeId>,
    Option<&'a Transform>,
);

/// Seeds new effect states from the scene seed and the light's stable key, then advances every
/// effect by one frame of virtual time.
pub(super) fn advance_light_effects(
    time: Res<Time>,
    settings: Res<DynamicLightingSettings>,
    paths: NamePaths,
    mut lights: Query<EffectItem>,
) {
    let delta = time.delta_secs();
    let now = time.elapsed_secs();
    for (entity, effect, mut state, bake_id, transform) in &mut lights {
        if !state.is_seeded() {
            let position = transform.map_or(Vec3::ZERO, |transform| transform.translation);
            let key = light_key(bake_id.copied(), &paths.name_path(entity), position);
            state.seed(settings.seed, key);
        }
        advance_effect(effect, &mut state, delta, now);
    }
}
