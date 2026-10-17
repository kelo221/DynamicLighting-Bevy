//! Chooses the [`ShaderFeatures`] of [`FeatureSelection::Automatic`] from what the scene uses.
//!
//! [`FeatureSelection::Automatic`]: super::settings::FeatureSelection::Automatic

use super::settings::ShaderFeatures;

/// Frames the scene must stay loaded before the features it uses are trusted. A bake applies,
/// re-tags its receivers and repacks its lights over a frame or two; until then the scene briefly
/// uses realtime lights and unbaked meshes.
pub const SETTLE_FRAMES: u32 = 5;

/// Frames to wait for a scene that keeps loading before choosing anyway (about 10 s at 60 fps).
pub const MAX_WAIT_FRAMES: u32 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// Nothing compiled yet; waiting for the scene to settle.
    Waiting { frames: u32, settled: u32 },
    /// `features` compiled; a newly used feature is added.
    Chosen(ShaderFeatures),
    /// `features` stay compiled while the scene reloads; chosen afresh once it settles.
    Reloading {
        features: ShaderFeatures,
        settled: u32,
    },
}

/// The automatic choice. Feed it every frame's use; it answers with the features to compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureChoice(State);

impl Default for FeatureChoice {
    fn default() -> Self {
        Self(State::Waiting {
            frames: 0,
            settled: 0,
        })
    }
}

impl FeatureChoice {
    /// One frame: the features the scene `used`, whether it was `loading`, and the features
    /// that may be compiled at all. Returns the features to compile, `None` until the first
    /// choice.
    pub fn update(
        &mut self,
        used: ShaderFeatures,
        loading: bool,
        allowed: ShaderFeatures,
    ) -> Option<ShaderFeatures> {
        let wanted = used.intersection(allowed);
        self.0 = match self.0 {
            State::Waiting { frames, settled } => {
                let settled = if loading { 0 } else { settled + 1 };
                if settled >= SETTLE_FRAMES || frames + 1 >= MAX_WAIT_FRAMES {
                    State::Chosen(wanted)
                } else {
                    State::Waiting {
                        frames: frames + 1,
                        settled,
                    }
                }
            }
            State::Chosen(features) if loading => State::Reloading {
                features,
                settled: 0,
            },
            State::Chosen(features) => State::Chosen(features.union(wanted)),
            State::Reloading { features, .. } if loading => State::Reloading {
                features,
                settled: 0,
            },
            State::Reloading { features, settled } => {
                if settled + 1 >= SETTLE_FRAMES {
                    State::Chosen(wanted)
                } else {
                    State::Reloading {
                        features,
                        settled: settled + 1,
                    }
                }
            }
        };
        match self.0 {
            State::Waiting { .. } => None,
            State::Chosen(features) | State::Reloading { features, .. } => {
                Some(features.intersection(allowed))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: ShaderFeatures = ShaderFeatures::ALL;
    const NONE: ShaderFeatures = ShaderFeatures::NONE;

    fn with(f: impl FnOnce(&mut ShaderFeatures)) -> ShaderFeatures {
        let mut features = NONE;
        f(&mut features);
        features
    }

    /// Runs `frames` frames of the same use and returns the last answer.
    fn run(
        choice: &mut FeatureChoice,
        frames: u32,
        used: ShaderFeatures,
        loading: bool,
    ) -> Option<ShaderFeatures> {
        (0..frames)
            .map(|_| choice.update(used, loading, ALL))
            .last()
            .flatten()
    }

    #[test]
    fn waits_while_the_scene_loads_then_chooses_what_it_uses() {
        let mut choice = FeatureChoice::default();
        let startup = with(|f| (f.realtime_lights, f.unbaked_meshes) = (true, true));
        assert_eq!(run(&mut choice, 30, startup, true), None);
        let baked = with(|f| f.bounce = true);
        assert_eq!(run(&mut choice, SETTLE_FRAMES - 1, baked, false), None);
        assert_eq!(choice.update(baked, false, ALL), Some(baked));
    }

    #[test]
    fn a_scene_that_never_finishes_loading_is_chosen_for_anyway() {
        let mut choice = FeatureChoice::default();
        let used = with(|f| f.area_lights = true);
        assert_eq!(run(&mut choice, MAX_WAIT_FRAMES - 1, used, true), None);
        assert_eq!(choice.update(used, true, ALL), Some(used));
    }

    #[test]
    fn a_feature_needed_later_is_added_and_kept() {
        let mut choice = FeatureChoice::default();
        run(&mut choice, SETTLE_FRAMES, NONE, false);
        let spot = with(|f| f.light_patterns = true);
        assert_eq!(choice.update(spot, false, ALL), Some(spot));
        assert_eq!(run(&mut choice, 100, NONE, false), Some(spot));
    }

    #[test]
    fn a_reload_keeps_the_features_until_it_settles_then_chooses_afresh() {
        let mut choice = FeatureChoice::default();
        let area = with(|f| f.area_lights = true);
        run(&mut choice, SETTLE_FRAMES, area, false);
        let loading = with(|f| f.realtime_lights = true);
        assert_eq!(run(&mut choice, 20, loading, true), Some(area));
        let bounce = with(|f| f.bounce = true);
        assert_eq!(
            run(&mut choice, SETTLE_FRAMES - 1, bounce, false),
            Some(area)
        );
        assert_eq!(choice.update(bounce, false, ALL), Some(bounce));
    }

    #[test]
    fn features_that_are_not_allowed_are_never_chosen() {
        let mut choice = FeatureChoice::default();
        let used = with(|f| (f.area_lights, f.bounce) = (true, true));
        let allowed = with(|f| f.bounce = true);
        let answers: Vec<_> = (0..SETTLE_FRAMES)
            .map(|_| choice.update(used, false, allowed))
            .collect();
        assert_eq!(answers.last().copied().flatten(), Some(allowed));
    }
}
