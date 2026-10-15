//! Compiles the surface WESL modules with naga, so shader errors fail `cargo test`.

use crate::baked_lighting::shader_tests::{BAKED_DATA, TRIANGLES};
use crate::dynamic_geometry::shader_tests::{DISTANCE_CUBES, LIGHT_BVH};
use crate::light::shader_tests::LIGHT_TYPES;
use crate::realtime_shadows::shader_tests::VSM;
use crate::shared::wesl_check::{MATERIAL_CONSTANTS as CONSTANTS, SHARED_TYPES, compile};

pub(crate) const BINDINGS: (&str, &str) = (
    "dynlight::surface::shaders::bindings",
    include_str!("shaders/bindings.wesl"),
);
pub(crate) const BRDF: (&str, &str) = (
    "dynlight::surface::shaders::brdf",
    include_str!("shaders/brdf.wesl"),
);
pub(crate) const LIGHT_PROCESSOR: (&str, &str) = (
    "dynlight::surface::shaders::light_processor",
    include_str!("shaders/light_processor.wesl"),
);
pub(crate) const LIGHTING: (&str, &str) = (
    "dynlight::surface::shaders::lighting",
    include_str!("shaders/lighting.wesl"),
);
pub(crate) const SHADE: (&str, &str) = (
    "dynlight::surface::shaders::shade",
    include_str!("shaders/shade.wesl"),
);

pub(crate) fn surface_modules() -> Vec<(&'static str, &'static str)> {
    vec![
        CONSTANTS,
        SHARED_TYPES,
        LIGHT_TYPES,
        BAKED_DATA,
        TRIANGLES,
        LIGHT_BVH,
        DISTANCE_CUBES,
        VSM,
        BINDINGS,
        BRDF,
        LIGHT_PROCESSOR,
        LIGHTING,
        SHADE,
    ]
}

fn has_function(module: &naga::Module, name: &str) -> bool {
    module
        .functions
        .iter()
        .any(|(_, function)| function.name.as_deref() == Some(name))
}

#[test]
fn lighting_compiles_and_validates() {
    let module = compile(LIGHTING.0, &surface_modules(), &[]);
    for name in [
        "surface_light",
        "dynamic_surface_light",
        "baked_surface_light",
    ] {
        assert!(has_function(&module, name), "{name} is missing");
    }
}

#[test]
fn shading_compiles_with_and_without_uvs() {
    for uvs in [false, true] {
        let module = compile(SHADE.0, &surface_modules(), &[("VERTEX_UVS_A", uvs)]);
        assert!(has_function(&module, "shade"));
        assert!(has_function(&module, "apply_normal_map"));
    }
}

#[test]
fn material_uniform_layout_matches_wesl() {
    use bevy::render::render_resource::ShaderType;

    use super::DynLitUniform;
    use crate::shared::wesl_check::struct_layout;

    let module = compile(BINDINGS.0, &surface_modules(), &[]);
    let layout = struct_layout(&module, "DynLitUniform");
    assert_eq!(u64::from(layout.size), DynLitUniform::min_size().get());
    let offsets: Vec<u32> = layout.members.iter().map(|(_, offset)| *offset).collect();
    assert_eq!(offsets, [0, 16, 32, 36, 40, 44, 48, 52]);
}

#[test]
fn material_flags_match_wesl() {
    use super::flags;
    let source = BINDINGS.1;
    for (name, value) in [
        ("FLAG_ALPHA_MASK", flags::ALPHA_MASK),
        ("FLAG_UNLIT", flags::UNLIT),
        ("FLAG_BASE_COLOR_TEXTURE", flags::BASE_COLOR_TEXTURE),
        ("FLAG_EMISSIVE_TEXTURE", flags::EMISSIVE_TEXTURE),
        (
            "FLAG_METALLIC_ROUGHNESS_TEXTURE",
            flags::METALLIC_ROUGHNESS_TEXTURE,
        ),
        ("FLAG_NORMAL_MAP", flags::NORMAL_MAP),
        ("FLAG_OCCLUSION_TEXTURE", flags::OCCLUSION_TEXTURE),
        ("FLAG_REFLECTION", flags::REFLECTION),
        ("FLAG_METALLIC_SHADING", flags::METALLIC_SHADING),
    ] {
        assert!(
            source.contains(&format!("const {name}: u32 = {value}u;")),
            "{name} differs"
        );
    }
}

#[test]
fn light_count_scale_matches_the_setting() {
    use crate::shared::DEBUG_LIGHT_COUNT_MAX;
    let line = format!("const DEBUG_LIGHT_COUNT_MAX: f32 = {DEBUG_LIGHT_COUNT_MAX}.0;");
    assert!(SHADE.1.contains(&line), "{line} is missing");
}
