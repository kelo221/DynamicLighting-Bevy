use bevy::prelude::*;

use super::*;
use crate::shared::{BakeId, DynamicLightingSettings};

/// Runs `effect` for `frames` frames of `delta` seconds from time 0 and returns the intensities.
fn trace(effect: LightEffect, seed: u64, delta: f32, frames: usize) -> Vec<f32> {
    let mut state = LightEffectState::seeded(seed, 1);
    (0..frames)
        .map(|frame| {
            advance_effect(&effect, &mut state, delta, frame as f32 * delta);
            state.intensity()
        })
        .collect()
}

#[test]
fn every_effect_stays_finite_and_in_range() {
    for kind in LightEffectKind::ALL {
        for delta in [1.0 / 30.0, 1.0 / 144.0, 0.25] {
            let samples = trace(LightEffect::new(kind), 5, delta, 4_000);
            let upper = if kind == LightEffectKind::Lightning {
                2.0
            } else {
                1.0
            };
            for (frame, value) in samples.iter().enumerate() {
                assert!(
                    value.is_finite() && (0.0..=upper).contains(value),
                    "{kind:?} delta {delta} frame {frame}: {value}"
                );
            }
        }
    }
}

#[test]
fn steady_is_full_intensity() {
    let samples = trace(LightEffect::new(LightEffectKind::Steady), 0, 1.0 / 60.0, 10);
    assert!(samples.iter().all(|&value| value == 1.0));
}

#[test]
fn pulse_swings_between_the_modifier_and_full() {
    let effect = LightEffect::new(LightEffectKind::Pulse);
    let mut state = LightEffectState::seeded(0, 0);
    advance_effect(&effect, &mut state, 0.0, 0.25);
    assert!((state.intensity() - 1.0).abs() < 1e-6);
    advance_effect(&effect, &mut state, 0.0, 0.75);
    assert!((state.intensity() - effect.pulse_modifier).abs() < 1e-6);
}

#[test]
fn strobe_toggles_once_per_step() {
    let effect = LightEffect::new(LightEffectKind::Strobe);
    let samples = trace(effect, 0, effect.timestep, 4);
    assert_eq!(samples, [1.0, 0.25, 1.0, 0.25]);

    let half_steps = trace(effect, 0, effect.timestep * 0.5, 5);
    assert_eq!(half_steps, [1.0, 0.25, 0.25, 1.0, 1.0]);
}

#[test]
fn random_stays_between_modifier_and_full_and_changes_only_on_steps() {
    let effect = LightEffect::new(LightEffectKind::Random);
    let samples = trace(effect, 9, effect.timestep / 4.0, 400);
    assert!(samples.iter().all(|value| (0.25..=1.0).contains(value)));
    let changes = samples.windows(2).filter(|pair| pair[0] != pair[1]).count();
    assert!(changes <= 100, "{changes} changes in 100 steps");
    assert!(changes >= 90, "{changes} changes in 100 steps");
}

#[test]
fn flicker_is_off_about_half_the_time() {
    let effect = LightEffect::new(LightEffectKind::Flicker);
    let samples = trace(effect, 3, effect.timestep, 4_000);
    let off = samples.iter().filter(|&&value| value == 0.0).count() as f32 / 4_000.0;
    assert!((0.45..0.55).contains(&off), "off share {off}");
    assert!(samples.iter().all(|&value| value == 0.0 || value >= 0.25));
}

#[test]
fn randomized_effects_repeat_per_seed_and_differ_between_seeds() {
    for kind in [
        LightEffectKind::Random,
        LightEffectKind::Flicker,
        LightEffectKind::FluorescentRandom,
    ] {
        let effect = LightEffect::new(kind);
        let a = trace(effect, 17, 1.0 / 30.0, 300);
        assert_eq!(a, trace(effect, 17, 1.0 / 30.0, 300), "{kind:?}");
        assert_ne!(a, trace(effect, 18, 1.0 / 30.0, 300), "{kind:?}");
    }
}

#[test]
fn fluorescent_random_visits_every_phase() {
    let effect = LightEffect::new(LightEffectKind::FluorescentRandom);
    let mut state = LightEffectState::seeded(2, 2);
    let mut seen = [false; 3];
    for frame in 0..6_000 {
        advance_effect(&effect, &mut state, 1.0 / 60.0, frame as f32 / 60.0);
        seen[state.fluorescent_phase as usize] = true;
    }
    assert_eq!(seen, [true; 3]);
}

#[test]
fn time_driven_effects_share_the_global_phase() {
    for kind in [
        LightEffectKind::Pulse,
        LightEffectKind::Generator,
        LightEffectKind::Overcast,
        LightEffectKind::Cloudy,
        LightEffectKind::FluorescentStarter,
        LightEffectKind::FluorescentClicker,
    ] {
        let effect = LightEffect::new(kind);
        let mut early = LightEffectState::seeded(0, 1);
        let mut late = LightEffectState::seeded(0, 2);
        for frame in 0..300 {
            advance_effect(&effect, &mut early, 1.0 / 60.0, frame as f32 / 60.0);
        }
        advance_effect(&effect, &mut late, 1.0 / 60.0, 299.0 / 60.0);
        assert_eq!(early.intensity(), late.intensity(), "{kind:?}");
    }
}

#[test]
fn candle_and_fire_never_drop_below_the_modifier() {
    for kind in [LightEffectKind::Candle, LightEffectKind::Fire] {
        let samples = trace(LightEffect::new(kind), 0, 1.0 / 60.0, 20_000);
        assert!(samples.iter().all(|&value| value >= 0.25), "{kind:?}");
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        assert!(mean > 0.4 && mean < 0.95, "{kind:?} mean {mean}");
    }
}

#[test]
fn lightning_flashes_occasionally() {
    let effect = LightEffect {
        pulse_speed: 3.0,
        ..LightEffect::new(LightEffectKind::Lightning)
    };
    let samples = trace(effect, 0, 1.0 / 60.0, 60 * 120);
    let flashing = samples.iter().filter(|&&value| value > 0.6).count() as f32;
    let share = flashing / samples.len() as f32;
    assert!(share > 0.005 && share < 0.4, "flash share {share}");
}

#[test]
fn effect_order_between_lights_does_not_matter() {
    let effects = [
        LightEffect::new(LightEffectKind::Random),
        LightEffect::new(LightEffectKind::Flicker),
        LightEffect::new(LightEffectKind::FluorescentRandom),
    ];
    let run = |order: [usize; 3]| {
        let mut states: Vec<_> = (0..3).map(|i| LightEffectState::seeded(4, i)).collect();
        for frame in 0..200 {
            for &i in &order {
                advance_effect(&effects[i], &mut states[i], 1.0 / 30.0, frame as f32 / 30.0);
            }
        }
        states
            .iter()
            .map(LightEffectState::intensity)
            .collect::<Vec<_>>()
    };
    assert_eq!(run([0, 1, 2]), run([2, 0, 1]));
}

#[test]
fn plugin_seeds_from_bake_id_and_advances_effects() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<DynamicLightingSettings>()
        .add_plugins(EffectsPlugin);
    let with_id = app
        .world_mut()
        .spawn((LightEffect::new(LightEffectKind::Random), BakeId(42)))
        .id();
    let steady = app
        .world_mut()
        .spawn(LightEffect::new(LightEffectKind::Steady))
        .id();
    app.update();

    let state = app.world().get::<LightEffectState>(with_id).unwrap();
    let mut expected = LightEffectState::seeded(0, 42);
    advance_effect(
        &LightEffect::new(LightEffectKind::Random),
        &mut expected,
        0.0,
        0.0,
    );
    assert_eq!(state.intensity(), expected.intensity());
    assert_eq!(
        app.world()
            .get::<LightEffectState>(steady)
            .unwrap()
            .intensity(),
        1.0
    );
}
