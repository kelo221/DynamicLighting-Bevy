//! The inspector panel: Feathers widgets that edit the [`Inspected`] light.

use bevy::feathers::controls::{FeathersButton, FeathersCheckbox, FeathersSlider};
use bevy::feathers::display::{caption, label};
use bevy::feathers::theme::ThemeBackgroundColor;
use bevy::feathers::tokens;
use bevy::prelude::*;
use bevy::ui::Checked;
use bevy::ui_widgets::{Activate, SliderPrecision, SliderValue, ValueChange};
use dynlight::bake::BakeStatus;
use dynlight::prelude::*;

use crate::{Inspected, PendingBake, request_bake};

/// Shows the light type on the cycling button.
#[derive(Component, Clone, Default)]
struct TypeCaption;

/// Shows the effect on the cycling button.
#[derive(Component, Clone, Default)]
struct EffectCaption;

/// Shows whether a bake is running.
#[derive(Component, Clone, Default)]
struct BakeCaption;

type Light<'w, 's> = Single<'w, 's, &'static mut DynamicLight, With<Inspected>>;

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, panel.spawn())
        .add_systems(Update, update_captions);
}

/// Stores a slider's new value so it moves, and returns it.
fn slide(change: &On<ValueChange<f32>>, commands: &mut Commands) -> f32 {
    commands
        .entity(change.source)
        .insert(SliderValue(change.value));
    change.value
}

fn check(change: &On<ValueChange<bool>>, commands: &mut Commands) -> bool {
    if change.value {
        commands.entity(change.source).insert(Checked);
    } else {
        commands.entity(change.source).remove::<Checked>();
    }
    change.value
}

fn panel() -> impl Scene {
    bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(8),
            top: px(8),
            width: px(260),
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            padding: UiRect::all(px(10)),
        }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        Children [
            @label("Inspected light")
            --
            @FeathersButton
            on(|_: On<Activate>, mut light: Light| {
                let next = (light.light_type as usize + 1) % LightType::ALL.len();
                light.light_type = LightType::ALL[next];
            })
            Children [ @caption("Type") TypeCaption ]
            --
            @FeathersButton
            on(|_: On<Activate>, light: Single<(Entity, Option<&LightEffect>), With<Inspected>>, mut commands: Commands| {
                let (entity, effect) = *light;
                let kind = effect.map_or(LightEffectKind::Steady, |effect| effect.kind);
                let index = LightEffectKind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
                let next = LightEffectKind::ALL[(index + 1) % LightEffectKind::ALL.len()];
                commands.entity(entity).insert(LightEffect::new(next));
            })
            Children [ @caption("Effect") EffectCaption ]
            --
            @label("Hue")
            --
            @FeathersSlider { @max: 360.0 }
            SliderValue(35.0)
            SliderPrecision(0)
            on(|change: On<ValueChange<f32>>, mut light: Light, mut commands: Commands| {
                light.color = Color::hsl(slide(&change, &mut commands), 0.7, 0.8);
            })
            --
            @label("Intensity")
            --
            @FeathersSlider { @max: 8.0 }
            SliderValue(2.5)
            SliderPrecision(2)
            on(|change: On<ValueChange<f32>>, mut light: Light, mut commands: Commands| {
                light.intensity = slide(&change, &mut commands);
            })
            --
            @label("Radius (m)")
            --
            @FeathersSlider { @min: 0.5, @max: 20.0 }
            SliderValue(9.0)
            SliderPrecision(1)
            on(|change: On<ValueChange<f32>>, mut light: Light, mut commands: Commands| {
                light.radius = slide(&change, &mut commands);
            })
            --
            @label("Cone angle (spot, discoball)")
            --
            @FeathersSlider { @min: 2.0, @max: 90.0 }
            SliderValue(40.0)
            SliderPrecision(0)
            on(|change: On<ValueChange<f32>>, mut light: Light, mut commands: Commands| {
                let outer = slide(&change, &mut commands);
                light.outer_cutoff = outer;
                light.inner_cutoff = outer * 0.85;
            })
            --
            @label("Size (rectangle, tube) (m)")
            --
            @FeathersSlider { @max: 3.0 }
            SliderValue(1.0)
            SliderPrecision(2)
            on(|change: On<ValueChange<f32>>, mut light: Light, mut commands: Commands| {
                let width = slide(&change, &mut commands);
                light.size = Vec2::new(width, width * 0.5);
            })
            --
            @label("Disk (sun) (°)")
            --
            @FeathersSlider { @max: 10.0 }
            SliderValue(0.53)
            SliderPrecision(2)
            on(|change: On<ValueChange<f32>>, mut light: Light, mut commands: Commands| {
                light.angular_diameter = slide(&change, &mut commands);
            })
            --
            @label("Falloff")
            --
            @FeathersSlider { @max: 4.0 }
            SliderValue(0.0)
            SliderPrecision(2)
            on(|change: On<ValueChange<f32>>, mut light: Light, mut commands: Commands| {
                light.falloff = slide(&change, &mut commands);
            })
            --
            @FeathersCheckbox { @caption: bsn! { @caption("Realtime (never baked)") } }
            on(|change: On<ValueChange<bool>>, mut light: Light, mut commands: Commands| {
                light.realtime = check(&change, &mut commands);
            })
            --
            @FeathersCheckbox { @caption: bsn! { @caption("Realtime shadow cubemap") } }
            on(|change: On<ValueChange<bool>>, mut light: Light, mut commands: Commands| {
                light.shadows = if check(&change, &mut commands) {
                    ShadowMode::Realtime
                } else {
                    ShadowMode::Raytraced
                };
            })
            --
            @label("Ambient")
            --
            @FeathersSlider { @max: 0.5 }
            SliderValue(0.125)
            SliderPrecision(3)
            on(|change: On<ValueChange<f32>>, mut settings: ResMut<DynamicLightingSettings>, mut commands: Commands| {
                settings.ambient = Color::srgba(1.0, 1.0, 1.0, slide(&change, &mut commands));
            })
            --
            @FeathersButton
            on(|_: On<Activate>, status: Res<BakeStatus>, mut pending: ResMut<PendingBake>| {
                if !status.is_running() {
                    request_bake(&mut pending);
                }
            })
            Children [ @caption("Bake") BakeCaption ]
        ]
    }
}

type Caption<'a> = (&'a mut Text, Has<TypeCaption>, Has<EffectCaption>);
type AnyCaption = Or<(With<TypeCaption>, With<EffectCaption>, With<BakeCaption>)>;

fn update_captions(
    light: Single<(&DynamicLight, Option<&LightEffect>), With<Inspected>>,
    status: Res<BakeStatus>,
    pending: Res<PendingBake>,
    mut captions: Query<Caption, AnyCaption>,
) {
    let (light, effect) = *light;
    let effect = effect.map_or(LightEffectKind::Steady, |effect| effect.kind);
    for (mut text, is_type, is_effect) in &mut captions {
        let caption = if is_type {
            format!("Type: {:?}", light.light_type)
        } else if is_effect {
            format!("Effect: {effect:?}")
        } else if status.is_running() || pending.is_pending() {
            "Baking...".to_owned()
        } else {
            "Bake (preview)".to_owned()
        };
        if text.0 != caption {
            text.0 = caption;
        }
    }
}
