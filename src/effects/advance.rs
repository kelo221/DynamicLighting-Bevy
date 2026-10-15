//! Advances one light's effect by one frame.

use super::continuous::{self as c, Frame};
use super::effect::{LightEffect, LightEffectKind, LightEffectState};
use crate::shared::math::lerp_clamped;

/// Advances `state` by one frame of `delta_seconds`, at global animation time `time_seconds`.
///
/// Continuous effects are evaluated every frame. The stepped effects (Random, Strobe, Flicker)
/// change once on the first frame and then once per frame in which at least one whole
/// `timestep` elapsed, however many steps that was.
pub fn advance_effect(
    effect: &LightEffect,
    state: &mut LightEffectState,
    delta_seconds: f32,
    time_seconds: f32,
) {
    state.timestep.seconds_per_step = effect.timestep;
    state.timestep.update(delta_seconds);

    let frame = Frame {
        delta: delta_seconds,
        time: time_seconds,
    };
    let continuous = match effect.kind {
        LightEffectKind::Steady => Some(1.0),
        LightEffectKind::Pulse => Some(c::pulse(effect, frame)),
        LightEffectKind::Candle => Some(c::candle(effect, state, frame)),
        LightEffectKind::Fire => Some(c::fire(effect, state, frame)),
        LightEffectKind::Generator => Some(c::generator(effect, frame)),
        LightEffectKind::Lightning => Some(c::lightning(effect, state, frame)),
        LightEffectKind::Pulsar => Some(c::pulsar(effect, state, frame)),
        LightEffectKind::FluorescentStarter => Some(c::fluorescent_starter(effect, frame)),
        LightEffectKind::FluorescentClicker => Some(c::fluorescent_clicker(effect, frame)),
        LightEffectKind::FluorescentRandom => Some(c::fluorescent_random(effect, state, frame)),
        LightEffectKind::Overcast => Some(c::overcast(effect, frame)),
        LightEffectKind::Cloudy => Some(c::cloudy(effect, frame)),
        LightEffectKind::Random | LightEffectKind::Strobe | LightEffectKind::Flicker => None,
    };
    if let Some(intensity) = continuous {
        state.intensity = intensity;
    }

    if state.timestep.pending_steps() > 0 || !state.initialized {
        state.initialized = true;
        step(effect, state);
    }
}

fn step(effect: &LightEffect, state: &mut LightEffectState) {
    let modifier = effect.pulse_modifier;
    match effect.kind {
        LightEffectKind::Flicker => {
            state.intensity = if state.rng.next_f32() < 0.5 {
                0.0
            } else {
                lerp_clamped(modifier, 1.0, state.rng.next_f32())
            };
        }
        LightEffectKind::Random => {
            state.intensity = lerp_clamped(modifier, 1.0, state.rng.next_f32());
        }
        LightEffectKind::Strobe => {
            state.strobe_on = !state.strobe_on;
            state.intensity = if state.strobe_on { 1.0 } else { modifier };
        }
        _ => {}
    }
}
