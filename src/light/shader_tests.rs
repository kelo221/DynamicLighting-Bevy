//! Compiles the light WESL modules with naga, so shader errors fail `cargo test`.

use super::LightType;
use crate::shared::ChannelFlags;
use crate::shared::wesl_check::{SHARED_TYPES, compile};

/// `(import path, source)` of the light-type WESL module, for other slices' shader tests.
pub(crate) const LIGHT_TYPES: (&str, &str) = (
    "dynlight::light::shaders::light_types",
    include_str!("shaders/light_types.wesl"),
);

/// `(import path, source)` of the area-light WESL module.
pub(crate) const AREA: (&str, &str) = (
    "dynlight::light::shaders::area",
    include_str!("shaders/area.wesl"),
);

fn has_function(module: &naga::Module, name: &str) -> bool {
    module
        .functions
        .iter()
        .any(|(_, function)| function.name.as_deref() == Some(name))
}

#[test]
fn light_types_compile_and_validate() {
    let module = compile(LIGHT_TYPES.0, &[SHARED_TYPES, LIGHT_TYPES], &[]);
    for name in [
        "attenuation",
        "pattern",
        "shimmer",
        "spot_theta",
        "discoball_theta",
    ] {
        assert!(has_function(&module, name), "{name} missing");
    }
}

#[test]
fn area_lights_compile_and_validate() {
    let module = compile(AREA.0, &[SHARED_TYPES, AREA], &[]);
    for name in [
        "area_diffuse",
        "area_representative_point",
        "area_specular_scale",
        "nearest_point",
    ] {
        assert!(has_function(&module, name), "{name} missing");
    }
}

#[test]
fn light_type_constants_match_the_enum() {
    for kind in LightType::ALL {
        let name = format!("{kind:?}").to_uppercase();
        let value = (kind as u32) << ChannelFlags::TYPE_SHIFT;
        let line = format!("const LIGHT_TYPE_{name}: u32 = {value}u;");
        assert!(SHARED_TYPES.1.contains(&line), "{line} is missing");
    }
}

#[test]
fn channel_masks_match_the_shader() {
    for (name, value) in [
        ("LIGHT_FLAG_REALTIME", ChannelFlags::REALTIME),
        ("LIGHT_TYPE_MASK", ChannelFlags::TYPE_MASK),
        ("LIGHT_SHIMMER_MASK", ChannelFlags::SHIMMER_MASK),
        ("LIGHT_FLAG_SHADOW_CUBEMAP", ChannelFlags::SHADOW_CUBEMAP),
        ("LIGHT_FLAG_COOKIE", ChannelFlags::COOKIE),
        ("LIGHT_BOUNCE_MASK", ChannelFlags::BOUNCE_MASK),
    ] {
        let line = format!("const {name}: u32 = {value}u;");
        assert!(SHARED_TYPES.1.contains(&line), "{line} is missing");
    }
    let read = format!(
        "(light.channel & LIGHT_BOUNCE_MASK) >> {}u)",
        ChannelFlags::BOUNCE_SHIFT
    );
    assert!(SHARED_TYPES.1.contains(&read), "{read} is missing");
}
