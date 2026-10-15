//! Compiles the realtime-shadow WESL library modules with naga.

use crate::shared::wesl_check::{MATERIAL_CONSTANTS, SHARED_TYPES, compile};

pub(crate) const VSM: (&str, &str) = (
    "dynlight::realtime_shadows::shaders::vsm",
    include_str!("shaders/vsm.wesl"),
);
const KERNEL: (&str, &str) = (
    "dynlight::realtime_shadows::shaders::shadow_kernel",
    include_str!("shaders/shadow_kernel.wesl"),
);

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
fn blur_weights_are_a_normalized_unit_gaussian() {
    let side = (-0.5_f32).exp();
    let (center, side) = (1.0 / (1.0 + 2.0 * side), side / (1.0 + 2.0 * side));
    let source = KERNEL.1;
    let weight = |name: &str| -> f32 {
        let line = source
            .lines()
            .find(|line| line.starts_with(&format!("const {name}: f32 = ")))
            .unwrap_or_else(|| panic!("{name} is missing"));
        line.rsplit(' ')
            .next()
            .unwrap()
            .trim_end_matches(';')
            .parse()
            .unwrap()
    };
    assert!((weight("BLUR_CENTER") - center).abs() < 1e-6);
    assert!((weight("BLUR_SIDE") - side).abs() < 1e-6);
}
