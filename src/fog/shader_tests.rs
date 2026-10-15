//! Compiles the fog WESL with naga and checks its constants against the Rust side.

use super::volumetric::VolumetricShape;
use crate::shared::wesl_check::{SHARED_TYPES, compile};

const FOG_VOLUMES: (&str, &str) = (
    "dynlight::fog::shaders::fog_volumes",
    include_str!("shaders/fog_volumes.wesl"),
);

fn has_function(module: &naga::Module, name: &str) -> bool {
    module
        .functions
        .iter()
        .any(|(_, function)| function.name.as_deref() == Some(name))
}

#[test]
fn fog_volumes_compile_and_validate() {
    let module = compile(FOG_VOLUMES.0, &[SHARED_TYPES, FOG_VOLUMES], &[]);
    for name in ["fog_opacity", "add_fog", "compose", "ray_box", "ray_cone"] {
        assert!(has_function(&module, name), "{name} is missing");
    }
}

#[test]
fn shape_constants_match_the_component() {
    let source = FOG_VOLUMES.1;
    for (name, shape) in [
        ("SPHERE", VolumetricShape::Sphere),
        ("BOX", VolumetricShape::Box),
        ("CONE_Z", VolumetricShape::ConeZ),
        ("CONE_Y", VolumetricShape::ConeY),
    ] {
        let line = format!("const SHAPE_{name}: u32 = {}u;", shape as u32);
        assert!(source.contains(&line), "missing `{line}`");
    }
}
