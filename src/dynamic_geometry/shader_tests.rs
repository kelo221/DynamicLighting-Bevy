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

fn modules() -> Vec<(&'static str, &'static str)> {
    vec![
        MATERIAL_CONSTANTS,
        SHARED_TYPES,
        BAKED_DATA,
        LIGHT_BVH,
        DISTANCE_CUBES,
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
    for name in ["cube_shadow", "cube_angular", "cube_trilinear"] {
        assert!(has_function(&module, name), "{name} is missing");
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
