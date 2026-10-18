//! Compiles the dynamic-geometry WESL modules with naga.

use crate::baked_lighting::shader_tests::BAKED_DATA;
use crate::shared::wesl_check::{MATERIAL_CONSTANTS, SHARED_TYPES, compile};

pub(crate) const LIGHT_BVH: (&str, &str) = (
    "dynlight::dynamic_geometry::shaders::light_bvh",
    include_str!("shaders/light_bvh.wesl"),
);
pub(crate) const DISTANCE_CUBES: (&str, &str) = (
    "dynlight::dynamic_geometry::shaders::distance_cubes",
    include_str!("shaders/distance_cubes.wesl"),
);
pub(crate) const SUN_MAP: (&str, &str) = (
    "dynlight::dynamic_geometry::shaders::sun_map",
    include_str!("shaders/sun_map.wesl"),
);

fn modules() -> Vec<(&'static str, &'static str)> {
    vec![
        MATERIAL_CONSTANTS,
        SHARED_TYPES,
        BAKED_DATA,
        LIGHT_BVH,
        DISTANCE_CUBES,
        SUN_MAP,
    ]
}

fn has_function(module: &naga::Module, name: &str) -> bool {
    module
        .functions
        .iter()
        .any(|(_, function)| function.name.as_deref() == Some(name))
}

#[test]
fn light_bvh_compiles_and_validates() {
    let module = compile(LIGHT_BVH.0, &modules(), &[]);
    assert!(has_function(&module, "bvh_start"));
    assert!(has_function(&module, "bvh_next"));
}

#[test]
fn distance_cubes_compile_and_validate() {
    let module = compile(DISTANCE_CUBES.0, &modules(), &[]);
    for name in [
        "cube_shadow",
        "cube_angular",
        "cube_trilinear",
        "cube_area_shadow",
    ] {
        assert!(has_function(&module, name), "{name} is missing");
    }
}

#[test]
fn sun_maps_compile_and_match_the_mirror() {
    use super::sun_mirror::{SUN_MAP_BIAS, SUN_MAP_TEXEL_BIAS};
    let module = compile(SUN_MAP.0, &modules(), &[]);
    assert!(has_function(&module, "sun_map_shadow"));
    assert!(has_function(&module, "sun_map_available"));
    for line in [
        format!("const SUN_MAP_BIAS: f32 = {SUN_MAP_BIAS};"),
        format!("const SUN_MAP_TEXEL_BIAS: f32 = {SUN_MAP_TEXEL_BIAS:?};"),
    ] {
        assert!(SUN_MAP.1.contains(&line), "{line}");
    }
}

#[test]
fn cube_constants_match_the_baker() {
    use crate::shared::cube::CUBE_FACE_COUNT;
    let source = DISTANCE_CUBES.1;
    assert!(source.contains("const CUBE_RESOLUTION: u32 = 64u;"));
    let words = 64 * 64 * CUBE_FACE_COUNT / 2;
    assert!(source.contains(&format!("const CUBE_WORDS: u32 = {words}u;")));
}

#[test]
fn soft_lookup_constants_match_the_mirror() {
    use super::mirror::*;
    let source = DISTANCE_CUBES.1;
    let expected = [
        format!("const CUBE_BIAS: f32 = {DISTANCE_CUBE_BIAS};"),
        format!("const BLOCKER_TAPS: u32 = {BLOCKER_TAPS}u;"),
        format!("const FILTER_TAPS: u32 = {FILTER_TAPS}u;"),
        format!("const SEARCH_SCALE: f32 = {SEARCH_SCALE:?};"),
        format!("const TEXEL_SPREAD: f32 = {TEXEL_SPREAD};"),
        format!("const GOLDEN_FRACTION: f32 = {GOLDEN_FRACTION};"),
        format!("const MAX_FILTER_LEVEL: f32 = {MAX_FILTER_LEVEL:?};"),
    ];
    for line in expected {
        assert!(source.contains(&line), "{line}");
    }
    assert_eq!(CUBE_RESOLUTION, crate::bake::DISTANCE_CUBE_RESOLUTION);
}
