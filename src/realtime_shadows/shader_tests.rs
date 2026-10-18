//! Compiles the realtime-shadow WESL library modules with naga, and checks their constants and
//! the face uniform against the CPU side.

use super::{mirror, sun_mirror};
use crate::shared::wesl_check::{MATERIAL_CONSTANTS, SHARED_TYPES, compile};

pub(crate) const VSM: (&str, &str) = (
    "dynlight::realtime_shadows::shaders::vsm",
    include_str!("shaders/vsm.wesl"),
);
pub(crate) const SUN_VSM: (&str, &str) = (
    "dynlight::realtime_shadows::shaders::sun_vsm",
    include_str!("shaders/sun_vsm.wesl"),
);
const KERNEL: (&str, &str) = (
    "dynlight::realtime_shadows::shaders::shadow_kernel",
    include_str!("shaders/shadow_kernel.wesl"),
);

/// The value of `const name: T = value;` in `source`, for `T` = `f32` or `i32`.
fn constant<T: core::str::FromStr>(source: &str, name: &str) -> T {
    let line = source
        .lines()
        .find(|line| {
            line.starts_with(&format!("const {name}: f32 = "))
                || line.starts_with(&format!("const {name}: i32 = "))
        })
        .unwrap_or_else(|| panic!("{name} is missing"));
    line.rsplit(' ')
        .next()
        .and_then(|value| value.trim_end_matches(';').parse().ok())
        .unwrap_or_else(|| panic!("{name} is not a number"))
}

#[test]
fn vsm_compiles_and_validates() {
    let module = compile(VSM.0, &[MATERIAL_CONSTANTS, SHARED_TYPES, VSM], &[]);
    assert!(
        module
            .functions
            .iter()
            .any(|(_, function)| function.name.as_deref() == Some("realtime_shadow"))
    );
}

#[test]
fn vsm_constants_match_the_mirror() {
    for (name, value) in [
        ("VSM_BIAS", mirror::VSM_BIAS),
        ("MIN_BLEEDING_CUT", mirror::MIN_BLEEDING_CUT),
        ("TERMINATOR_START", mirror::TERMINATOR_START),
        ("TERMINATOR_END", mirror::TERMINATOR_END),
    ] {
        assert_eq!(constant::<f32>(VSM.1, name), value, "{name}");
    }
}

#[test]
fn sun_vsm_compiles_and_matches_the_mirror() {
    let modules = [MATERIAL_CONSTANTS, SHARED_TYPES, VSM, SUN_VSM];
    let module = compile(SUN_VSM.0, &modules, &[]);
    assert!(
        module
            .functions
            .iter()
            .any(|(_, function)| function.name.as_deref() == Some("sun_shadow"))
    );
    for (name, value) in [
        ("SUN_NORMAL_OFFSET", sun_mirror::SUN_NORMAL_OFFSET),
        ("SUN_CASCADE_MARGIN", sun_mirror::SUN_CASCADE_MARGIN),
        ("SUN_CASCADE_BLEND", sun_mirror::SUN_CASCADE_BLEND),
    ] {
        assert_eq!(constant::<f32>(SUN_VSM.1, name), value, "{name}");
    }
}

#[test]
fn blur_kernel_compiles_and_matches_the_mirror() {
    let module = compile(KERNEL.0, &[KERNEL], &[]);
    for name in ["blur_taps", "blur_weight"] {
        assert!(
            module
                .functions
                .iter()
                .any(|(_, function)| function.name.as_deref() == Some(name)),
            "{name}"
        );
    }
    assert_eq!(
        constant::<i32>(KERNEL.1, "MAX_BLUR_TAPS"),
        mirror::MAX_BLUR_TAPS
    );
    // The uniform the passes read: radius, layer, blur and the cascade flag.
    use bevy::render::render_resource::ShaderType;
    assert_eq!(super::ShadowFaceUniform::min_size().get(), 16);
}
