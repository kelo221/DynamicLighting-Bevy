//! Effects evaluated every frame from the global time.

use core::f32::consts::{PI, TAU};

use super::{effect::LightEffect, effect::LightEffectState, noise::noise01};
use crate::shared::math::{lerp_clamped, move_towards, smoothstep01};

/// Per-frame inputs shared by every continuous effect.
#[derive(Clone, Copy)]
pub(super) struct Frame {
    /// Seconds since the previous frame.
    pub(super) delta: f32,
    /// Global animation time in seconds, the same for every light.
    pub(super) time: f32,
}

pub(super) fn pulse(effect: &LightEffect, frame: Frame) -> f32 {
    let phase = (effect.pulse_offset + frame.time * effect.pulse_speed) * TAU;
    lerp_clamped(effect.pulse_modifier, 1.0, (1.0 + phase.sin()) * 0.5)
}

pub(super) fn candle(effect: &LightEffect, state: &LightEffectState, frame: Frame) -> f32 {
    let modifier = effect.pulse_modifier;
    let t = effect.pulse_offset + frame.time;
    let baseline = noise01(t * 0.5, 0.0);
    let dip = noise01(t * 2.0, 10.0);
    let sharp_dip = if dip > 0.8 { (dip - 0.8) * 2.0 } else { 0.0 };
    let mut target = lerp_clamped(modifier, 1.0, baseline - sharp_dip);
    if noise01(t * 5.0, 20.0) > 0.7 {
        target *= modifier;
    }
    move_towards(state.intensity, target, frame.delta).max(modifier)
}

pub(super) fn fire(effect: &LightEffect, state: &LightEffectState, frame: Frame) -> f32 {
    let modifier = effect.pulse_modifier;
    let t = effect.pulse_offset + frame.time;
    let baseline = noise01(t * 0.5, 0.0);
    let chaotic = noise01(t * 3.0, 10.0);
    let mut target = lerp_clamped(modifier, 1.0, baseline * 0.6 + chaotic * 0.4);
    if noise01(t * 5.0, 20.0) > 0.7 {
        target = 1.0;
    }
    move_towards(state.intensity, target, frame.delta * 3.0).max(modifier)
}

pub(super) fn generator(effect: &LightEffect, frame: Frame) -> f32 {
    let t = effect.pulse_offset + frame.time * effect.pulse_speed;
    let sag = (-1.0 + noise01(t, t * 0.1) * 2.5).clamp(-1.0, 0.0);
    1.0 + sag * (1.0 - effect.pulse_modifier)
}

pub(super) fn lightning(effect: &LightEffect, state: &LightEffectState, frame: Frame) -> f32 {
    let modifier = effect.pulse_modifier;
    let t = frame.time + effect.pulse_offset;
    let slow_wave = 0.5 + (t * TAU).sin() * 0.5;
    let mut intensity = move_towards(state.intensity, modifier, frame.delta * 10.0 * slow_wave);
    let layers = effect.pulse_speed.abs().round() as i32;
    lightning_layer(&mut intensity, modifier, t, 0.2);
    if layers >= 2 {
        lightning_layer(&mut intensity, modifier, t + 121.81, 4.5);
    }
    if layers >= 3 {
        lightning_layer(&mut intensity, modifier, t + 281.24, 7.8);
    }
    intensity
}

fn lightning_layer(intensity: &mut f32, modifier: f32, t: f32, unique: f32) {
    let slow_wave = 0.5 + ((t + unique) * TAU).sin() * 0.5;
    let remainder = (1.0 - modifier) * slow_wave;
    if noise01(t * 5.0, unique) > 0.74 {
        *intensity = modifier + remainder + (t * PI * 40.0).sin() * remainder;
    }
}

pub(super) fn pulsar(effect: &LightEffect, state: &LightEffectState, frame: Frame) -> f32 {
    let t = effect.pulse_offset + frame.time;
    let phase = (t * TAU * effect.pulse_speed).sin().max(0.0);
    let target = lerp_clamped(effect.pulse_modifier, 1.0, phase.powf(5.0));
    move_towards(state.intensity, target, frame.delta * 2.0)
}

/// 50 Hz ballast hum while a fluorescent tube pre-heats.
fn ballast_hum(t: f32, modifier: f32) -> f32 {
    (0.25 + (t * PI * 50.0).sin() * 0.125) * (1.0 - modifier)
}

pub(super) fn fluorescent_starter(effect: &LightEffect, frame: Frame) -> f32 {
    let sequence = (effect.pulse_offset + frame.time).rem_euclid(3.3);
    if sequence < 0.5 {
        ballast_hum(sequence, effect.pulse_modifier)
    } else if sequence > 2.95 {
        lerp_clamped(1.0, 0.0, (sequence - 3.0) * 20.0)
    } else {
        1.0
    }
}

pub(super) fn fluorescent_clicker(effect: &LightEffect, frame: Frame) -> f32 {
    let sequence = (effect.pulse_offset + frame.time).rem_euclid(6.0);
    if sequence < 0.3 {
        0.1 + (sequence * PI * 20.0).sin() * 0.05
    } else if sequence < 1.5 {
        if sequence.rem_euclid(0.2) < 0.05 {
            1.0
        } else {
            0.0
        }
    } else if sequence < 4.5 {
        1.0
    } else {
        lerp_clamped(1.0, 0.0, (sequence - 4.5) / 0.0625)
    }
}

pub(super) fn fluorescent_random(
    effect: &LightEffect,
    state: &mut LightEffectState,
    frame: Frame,
) -> f32 {
    if frame.time > state.fluorescent_until {
        state.fluorescent_phase = state.rng.below(3);
        state.fluorescent_until = frame.time + state.rng.next_f32();
    }
    match state.fluorescent_phase {
        0 => move_towards(state.intensity, effect.pulse_modifier, frame.delta * 10.0),
        1 => ballast_hum(frame.time, effect.pulse_modifier),
        _ => move_towards(state.intensity, 1.0, frame.delta * 20.0),
    }
}

pub(super) fn overcast(effect: &LightEffect, frame: Frame) -> f32 {
    let t0 = effect.pulse_offset + frame.time * effect.pulse_speed;
    let mut noise = 0.0;
    let mut frequency = 1.0;
    let mut amplitude = 1.0;
    for _ in 0..3 {
        let t = t0 * frequency;
        noise += noise01(t * 0.1, t * 0.01) * amplitude;
        frequency *= 2.0;
        amplitude *= 0.5;
    }
    lerp_clamped(effect.pulse_modifier, 1.0, smoothstep01(noise / 1.75))
}

pub(super) fn cloudy(effect: &LightEffect, frame: Frame) -> f32 {
    let t0 = effect.pulse_offset + frame.time * effect.pulse_speed;
    let mut noise = 0.0;
    let mut frequency = 1.0;
    let mut amplitude = 1.0;
    for _ in 0..2 {
        let t = t0 * frequency;
        noise += ((noise01(t * 0.05, t * 0.005) - 0.5) * 4.0).clamp(0.0, 1.0) * amplitude;
        frequency *= 2.0;
        amplitude *= 0.5;
    }
    lerp_clamped(effect.pulse_modifier, 1.0, smoothstep01(1.0 - noise / 1.5))
}
