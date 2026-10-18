//! Compiles the environment WESL modules with naga.

use crate::shared::wesl_check::{SHARED_TYPES, compile};

pub(crate) const SKY: (&str, &str) = (
    "dynlight::environment::shaders::sky",
    include_str!("shaders/sky.wesl"),
);
const BACKGROUND: (&str, &str) = (
    "dynlight::environment::shaders::background",
    include_str!("shaders/background.wesl"),
);
/// The parts of Bevy's view module the background reads.
const VIEW: (&str, &str) = (
    "bevy_render::view",
    "struct View {
    world_from_view: mat4x4<f32>,
    view_from_clip: mat4x4<f32>,
    main_pass_viewport: vec4<f32>,
}
",
);

fn has_function(module: &naga::Module, name: &str) -> bool {
    module
        .functions
        .iter()
        .any(|(_, function)| function.name.as_deref() == Some(name))
}

#[test]
fn sky_compiles_and_validates() {
    let module = compile(SKY.0, &[SHARED_TYPES, SKY], &[]);
    for name in [
        "sky_local",
        "sky_irradiance",
        "sky_equirect_uv",
        "sky_background_lod",
    ] {
        assert!(has_function(&module, name), "{name} is missing");
    }
}

#[test]
fn background_compiles_and_validates() {
    let module = compile(BACKGROUND.0, &[SHARED_TYPES, SKY, VIEW, BACKGROUND], &[]);
    let entries: Vec<&str> = module
        .entry_points
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(entries, ["vertex", "fragment"]);
}

#[test]
fn the_basis_matches_the_cpu() {
    let source = SKY.1;
    for constant in ["0.282095", "0.488603", "1.092548", "0.315392", "0.546274"] {
        assert!(source.contains(constant), "{constant}");
    }
}
