//! Frame-rate independent stepping for the stepped effects (Random, Strobe, Flicker).

/// Smallest allowed step, matching the lower bound Henry puts on the timestep field.
const MIN_STEP_SECONDS: f32 = 1.0e-5;

/// Accumulates frame time and reports how many whole steps elapsed during the last update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FixedTimestep {
    pub seconds_per_step: f32,
    accumulator: f32,
    pending_steps: u32,
}

impl FixedTimestep {
    pub const fn new(seconds_per_step: f32) -> Self {
        Self {
            seconds_per_step,
            accumulator: 0.0,
            pending_steps: 0,
        }
    }

    pub fn update(&mut self, delta_seconds: f32) {
        let step = self.seconds_per_step.max(MIN_STEP_SECONDS);
        self.pending_steps = 0;
        self.accumulator += delta_seconds.max(0.0);
        if self.accumulator >= step {
            self.pending_steps = (self.accumulator / step) as u32;
            self.accumulator -= self.pending_steps as f32 * step;
        }
    }

    /// Whole steps completed by the last [`Self::update`].
    pub fn pending_steps(&self) -> u32 {
        self.pending_steps
    }
}

impl Default for FixedTimestep {
    fn default() -> Self {
        Self::new(1.0 / 30.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_whole_steps_and_keeps_the_remainder() {
        let mut timestep = FixedTimestep::new(0.1);
        timestep.update(0.05);
        assert_eq!(timestep.pending_steps(), 0);
        timestep.update(0.26);
        assert_eq!(timestep.pending_steps(), 3);
        timestep.update(0.0);
        assert_eq!(timestep.pending_steps(), 0);
        timestep.update(0.095);
        assert_eq!(timestep.pending_steps(), 1);
    }

    #[test]
    fn degenerate_inputs_do_not_explode() {
        let mut timestep = FixedTimestep::new(0.0);
        timestep.update(1.0e-3);
        assert_eq!(timestep.pending_steps(), 100);
        timestep.update(-5.0);
        assert_eq!(timestep.pending_steps(), 0);
    }
}
