//! The effect authoring component and its per-light runtime state.

use bevy::prelude::*;

use super::{fixed_timestep::FixedTimestep, rng::EffectRng};

/// Animates the intensity of a light over time. Add it next to a `DynamicLight`; lights without
/// one shine steadily.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default, Debug, PartialEq, Clone)]
#[require(LightEffectState)]
pub struct LightEffect {
    pub kind: LightEffectKind,
    /// Speed of Pulse, Pulsar, Generator, Cloudy and Overcast. For Lightning, the number of
    /// overlapping flash layers (1 to 3).
    pub pulse_speed: f32,
    /// The lowest intensity multiplier the effect dips to, 0 to 1.
    pub pulse_modifier: f32,
    /// Time offset in seconds, so identical lights don't animate in sync.
    pub pulse_offset: f32,
    /// Seconds per step for the stepped effects: Random, Strobe and Flicker.
    pub timestep: f32,
}

impl Default for LightEffect {
    fn default() -> Self {
        Self {
            kind: LightEffectKind::Steady,
            pulse_speed: 1.0,
            pulse_modifier: 0.25,
            pulse_offset: 0.0,
            timestep: 1.0 / 30.0,
        }
    }
}

impl LightEffect {
    pub fn new(kind: LightEffectKind) -> Self {
        Self {
            kind,
            ..Default::default()
        }
    }
}

/// The intensity animations from Henry's package.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum LightEffectKind {
    /// Constant full intensity.
    #[default]
    Steady,
    /// Smooth sine between the modifier and full intensity.
    Pulse,
    /// A new random intensity every step.
    Random,
    /// Alternates between full intensity and the modifier every step.
    Strobe,
    /// Every step: off half the time, otherwise a random intensity.
    Flicker,
    /// A fluorescent tube that struggles to start, then stays on.
    FluorescentStarter,
    /// A fluorescent tube that clicks on and off in a fixed rhythm.
    FluorescentClicker,
    /// A broken fluorescent tube switching randomly between dim, buzzing and on.
    FluorescentRandom,
    /// A gently wavering candle with occasional dips.
    Candle,
    /// Short bright bursts.
    Pulsar,
    /// A lively flame with occasional flares.
    Fire,
    /// A generator-powered lamp that sags under load.
    Generator,
    /// Thunderstorm flashes; `pulse_speed` sets the number of layers.
    Lightning,
    /// Sunlight with clouds passing by.
    Cloudy,
    /// A dim, slowly shifting overcast sky.
    Overcast,
}

impl LightEffectKind {
    pub const ALL: [Self; 15] = [
        Self::Steady,
        Self::Pulse,
        Self::Random,
        Self::Strobe,
        Self::Flicker,
        Self::FluorescentStarter,
        Self::FluorescentClicker,
        Self::FluorescentRandom,
        Self::Candle,
        Self::Pulsar,
        Self::Fire,
        Self::Generator,
        Self::Lightning,
        Self::Cloudy,
        Self::Overcast,
    ];
}

/// Runtime state of a [`LightEffect`], advanced once per frame.
#[derive(Component, Clone, Debug, Default)]
pub struct LightEffectState {
    pub(super) intensity: f32,
    pub(super) initialized: bool,
    pub(super) timestep: FixedTimestep,
    pub(super) strobe_on: bool,
    pub(super) fluorescent_phase: u32,
    pub(super) fluorescent_until: f32,
    pub(super) rng: EffectRng,
    pub(super) seeded: bool,
}

impl LightEffectState {
    /// A state whose random stream is fixed by `seed` and `light_key`.
    pub fn seeded(seed: u64, light_key: u64) -> Self {
        let mut state = Self::default();
        state.seed(seed, light_key);
        state
    }

    /// Restarts the random stream. Done automatically from the scene seed and the light's
    /// `BakeId` (or entity) the first time the effect runs.
    pub fn seed(&mut self, seed: u64, light_key: u64) {
        self.rng = EffectRng::for_light(seed, light_key);
        self.seeded = true;
    }

    pub fn is_seeded(&self) -> bool {
        self.seeded
    }

    /// The current intensity multiplier. Usually 0 to 1; Lightning flashes can exceed 1.
    pub fn intensity(&self) -> f32 {
        self.intensity
    }
}
