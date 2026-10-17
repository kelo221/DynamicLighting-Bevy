//! Which [`ShaderFeatures`] the scene uses, and a warning when it keeps using one that
//! `DynamicLightingSettings::shader_features` switched off.

use bevy::prelude::*;

use super::settings::{DynamicLightingSettings, ShaderFeatures};

/// Seconds a switched-off feature must stay in use before the warning. A scene uses realtime
/// lights and unbaked meshes for a moment at startup, until its bake loads.
const WARN_AFTER_SECONDS: f32 = 3.0;

/// The lighting paths the scene used this frame. The slices that know set their features in
/// `PostUpdate`; it is cleared after the check in `Last`.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderFeaturesInUse(pub ShaderFeatures);

impl Default for ShaderFeaturesInUse {
    fn default() -> Self {
        Self(ShaderFeatures::NONE)
    }
}

/// Since when each switched-off feature has been in use, and which were reported.
#[derive(Resource, Default, Debug)]
pub(super) struct MissingFeatures {
    since: [Option<f32>; ShaderFeatures::COUNT],
    reported: [bool; ShaderFeatures::COUNT],
}

impl MissingFeatures {
    /// The features to report at time `now`: switched off, in use for `WARN_AFTER_SECONDS`
    /// without a break, and not reported before.
    pub(super) fn update(
        &mut self,
        used: ShaderFeatures,
        enabled: ShaderFeatures,
        now: f32,
    ) -> Vec<&'static str> {
        let mut report = Vec::new();
        for (i, ((name, used), (_, enabled))) in
            used.list().into_iter().zip(enabled.list()).enumerate()
        {
            if !used || enabled {
                self.since[i] = None;
                continue;
            }
            let since = *self.since[i].get_or_insert(now);
            if now - since >= WARN_AFTER_SECONDS && !self.reported[i] {
                self.reported[i] = true;
                report.push(name);
            }
        }
        report
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<ShaderFeaturesInUse>()
        .init_resource::<MissingFeatures>()
        .add_systems(Last, report_missing_features);
}

fn report_missing_features(
    settings: Res<DynamicLightingSettings>,
    time: Res<Time<Real>>,
    mut in_use: ResMut<ShaderFeaturesInUse>,
    mut missing: ResMut<MissingFeatures>,
) {
    let used = std::mem::take(&mut *in_use).0;
    for name in missing.update(used, settings.shader_features, time.elapsed_secs()) {
        warn!(
            "dynlight: the scene uses {name}, which DynamicLightingSettings::shader_features \
             leaves out of the lit shader; switch it on to light them correctly"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn realtime() -> ShaderFeatures {
        ShaderFeatures {
            realtime_lights: true,
            ..ShaderFeatures::NONE
        }
    }

    #[test]
    fn reports_a_missing_feature_once_after_the_delay() {
        let mut missing = MissingFeatures::default();
        let none = ShaderFeatures::NONE;
        assert!(missing.update(realtime(), none, 0.0).is_empty());
        assert!(missing.update(realtime(), none, 2.9).is_empty());
        assert_eq!(missing.update(realtime(), none, 3.0), ["realtime_lights"]);
        assert!(missing.update(realtime(), none, 10.0).is_empty());
    }

    #[test]
    fn a_break_restarts_the_delay() {
        let mut missing = MissingFeatures::default();
        let none = ShaderFeatures::NONE;
        missing.update(realtime(), none, 0.0);
        missing.update(none, none, 2.0);
        assert!(missing.update(realtime(), none, 4.0).is_empty());
        assert_eq!(missing.update(realtime(), none, 7.0), ["realtime_lights"]);
    }

    #[test]
    fn enabled_features_are_never_reported() {
        let mut missing = MissingFeatures::default();
        missing.update(realtime(), ShaderFeatures::ALL, 0.0);
        assert!(
            missing
                .update(realtime(), ShaderFeatures::ALL, 9.0)
                .is_empty()
        );
    }
}
