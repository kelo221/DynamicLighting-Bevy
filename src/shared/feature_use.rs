//! Which [`ShaderFeatures`] the scene uses, which ones the lit shader gets, and a warning when
//! the scene keeps using one that `DynamicLightingSettings::shader_features` switched off.

use bevy::prelude::*;

use super::feature_choice::FeatureChoice;
use super::settings::{DynamicLightingSettings, FeatureSelection, ShaderFeatures};

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

/// Set by the slices that know, in `PostUpdate`, for every frame in which the scene is still
/// loading data that decides [`ShaderFeaturesInUse`], such as a bake not yet applied. Cleared
/// with it in `Last`.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneLoading(pub bool);

/// The features compiled into the lit shader: `shader_features`, or the automatic choice.
/// `None` while [`FeatureSelection::Automatic`] waits for the scene; the lit shader is not
/// compiled until then.
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LitShaderFeatures(pub Option<ShaderFeatures>);

pub(super) fn plugin(app: &mut App) {
    let settings = app.world().resource::<DynamicLightingSettings>();
    let features = match settings.feature_selection {
        FeatureSelection::Fixed => Some(settings.shader_features),
        FeatureSelection::Automatic => None,
    };
    app.init_resource::<ShaderFeaturesInUse>()
        .init_resource::<SceneLoading>()
        .init_resource::<MissingFeatures>()
        .init_resource::<AutomaticChoice>()
        .insert_resource(LitShaderFeatures(features))
        .add_systems(Last, choose_features);
}

#[derive(Resource, Default)]
pub(super) struct AutomaticChoice(FeatureChoice);

/// Updates [`LitShaderFeatures`] from this frame's use, warns about features the scene keeps
/// using that `shader_features` leaves out, and clears the frame's use.
pub(super) fn choose_features(
    settings: Res<DynamicLightingSettings>,
    time: Res<Time<Real>>,
    mut in_use: ResMut<ShaderFeaturesInUse>,
    mut loading: ResMut<SceneLoading>,
    mut choice: ResMut<AutomaticChoice>,
    mut lit: ResMut<LitShaderFeatures>,
    mut missing: ResMut<MissingFeatures>,
) {
    let used = std::mem::take(&mut *in_use).0;
    let loading = std::mem::take(&mut *loading).0;
    let features = match settings.feature_selection {
        FeatureSelection::Fixed => {
            choice.0 = FeatureChoice::default();
            Some(settings.shader_features)
        }
        FeatureSelection::Automatic => choice.0.update(used, loading, settings.shader_features),
    };
    lit.set_if_neq(LitShaderFeatures(features));
    // Nothing is drawn lit before the first choice, so nothing is missing yet.
    let enabled = features.unwrap_or(ShaderFeatures::ALL);
    for name in missing.update(used, enabled, time.elapsed_secs()) {
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

    /// Runs a frame in which the scene used `used` and was `loading`.
    fn frame(app: &mut App, used: ShaderFeatures, loading: bool) -> Option<ShaderFeatures> {
        app.world_mut().resource_mut::<ShaderFeaturesInUse>().0 = used;
        app.world_mut().resource_mut::<SceneLoading>().0 = loading;
        app.update();
        app.world().resource::<LitShaderFeatures>().0
    }

    #[test]
    fn automatic_selection_waits_for_the_scene_then_follows_its_use() {
        use super::super::feature_choice::SETTLE_FRAMES;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DynamicLightingSettings {
                feature_selection: FeatureSelection::Automatic,
                ..default()
            });
        plugin(&mut app);
        assert_eq!(frame(&mut app, realtime(), true), None);
        let bounce = ShaderFeatures {
            bounce: true,
            ..ShaderFeatures::NONE
        };
        for _ in 1..SETTLE_FRAMES {
            assert_eq!(frame(&mut app, bounce, false), None);
        }
        assert_eq!(frame(&mut app, bounce, false), Some(bounce));
        let both = bounce.union(realtime());
        assert_eq!(frame(&mut app, realtime(), false), Some(both));

        app.world_mut()
            .resource_mut::<DynamicLightingSettings>()
            .feature_selection = FeatureSelection::Fixed;
        assert_eq!(frame(&mut app, bounce, false), Some(ShaderFeatures::ALL));
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
