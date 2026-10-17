//! Light intensity effects (candle, fire, strobe, fluorescent, lightning, …), with our own noise
//! and random numbers.
//!
//! Add [`LightEffect`] next to a light; [`LightEffectState::intensity`] holds the multiplier
//! for the current frame once [`EffectsSystems`] has run.

mod advance;
mod continuous;
mod effect;
mod fixed_timestep;
mod noise;
mod rng;
mod systems;
#[cfg(test)]
mod tests;

use bevy::prelude::*;

pub use advance::advance_effect;
pub use effect::{LightEffect, LightEffectKind, LightEffectState};
pub use fixed_timestep::FixedTimestep;
pub use noise::noise01;
pub use rng::{EffectRng, light_key, splitmix64};

/// Runs in `PostUpdate`. Systems that read effect intensities order themselves after this.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EffectsSystems;

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            systems::advance_light_effects.in_set(EffectsSystems),
        );
    }
}
