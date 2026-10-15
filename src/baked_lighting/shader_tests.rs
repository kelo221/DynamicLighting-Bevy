//! Compiles the baked-lighting WESL modules with naga and checks their constants against
//! `layout.rs`.

use super::layout::{AUX_HEADER_WORDS, RECEIVER_WORDS, flags, header};
use crate::shared::wesl_check::{MATERIAL_CONSTANTS, SHARED_TYPES, compile};

pub(crate) const BAKED_DATA: (&str, &str) = (
    "dynlight::baked_lighting::shaders::baked_data",
    include_str!("shaders/baked_data.wesl"),
);
pub(crate) const TRIANGLES: (&str, &str) = (
    "dynlight::baked_lighting::shaders::triangles",
    include_str!("shaders/triangles.wesl"),
);

fn modules() -> Vec<(&'static str, &'static str)> {
    vec![MATERIAL_CONSTANTS, SHARED_TYPES, BAKED_DATA, TRIANGLES]
}

#[test]
fn triangles_compile_and_validate() {
    let module = compile(TRIANGLES.0, &modules(), &[]);
    for name in [
        "load_triangle",
        "triangle_light",
        "baked_shadow",
        "baked_bounce",
    ] {
        assert!(
            module
                .functions
                .iter()
                .any(|(_, function)| function.name.as_deref() == Some(name)),
            "{name} is missing"
        );
    }
}

/// `const NAME: u32 = <n>u;` from a WESL source.
fn wesl_const(source: &str, name: &str) -> u32 {
    let prefix = format!("const {name}: u32 = ");
    let line = source
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("{name} is not declared"));
    line.trim_end_matches(';')
        .trim_end_matches('u')
        .parse()
        .unwrap_or_else(|_| panic!("{name} is not a u32 literal"))
}

#[test]
fn constants_match_the_layout() {
    let source = BAKED_DATA.1;
    let pairs = [
        ("AUX_RECEIVERS", header::RECEIVERS as u32),
        ("AUX_RECEIVER_COUNT", header::RECEIVER_COUNT as u32),
        ("AUX_BVH", header::BVH as u32),
        ("AUX_BVH_NODE_COUNT", header::BVH_NODE_COUNT as u32),
        ("AUX_CUBES", header::CUBES as u32),
        ("AUX_CUBE_COUNT", header::CUBE_COUNT as u32),
        ("AUX_FLAGS", header::FLAGS as u32),
        ("AUX_BAKED_LIGHTS", header::BAKED_LIGHTS as u32),
        ("AUX_FLAG_BOUNCE_LAYOUT", flags::BOUNCE_LAYOUT),
        ("AUX_FLAG_CUBE_BIAS", flags::CUBE_BIAS),
        ("AUX_FLAG_CUBE_ANGULAR", flags::CUBE_ANGULAR),
        ("RECEIVER_WORDS", RECEIVER_WORDS as u32),
    ];
    for (name, value) in pairs {
        assert_eq!(wesl_const(source, name), value, "{name}");
    }
    const { assert!(header::BAKED_LIGHTS < AUX_HEADER_WORDS) };
}
