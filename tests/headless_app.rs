//! The plugin works in a headless app (no window, renderer or asset server), as used by servers,
//! bake tools and tests.

use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use dynlight::prelude::*;

#[test]
fn plugin_runs_without_a_renderer() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin))
        .add_plugins(DynamicLightingPlugin);

    let light = app
        .world_mut()
        .spawn((
            DynamicLight::default(),
            LightEffect::new(LightEffectKind::Strobe),
            VolumetricLight::default(),
            BakeId::from_path(["Hall", "Lamp"]),
        ))
        .id();
    let plain = app.world_mut().spawn(DynamicLight::default()).id();

    for _ in 0..3 {
        app.update();
    }

    let world = app.world();
    assert!(world.contains_resource::<DynamicLightingSettings>());
    assert!(world.contains_resource::<BakeSettings>());
    assert!(
        world.get::<Transform>(plain).is_some(),
        "required components"
    );
    assert!(
        world.get::<Visibility>(plain).is_some(),
        "required components"
    );
    let state = world.get::<LightEffectState>(light).expect("effect state");
    assert!(state.is_seeded());
    assert!([1.0, 0.25].contains(&state.intensity()));
}
