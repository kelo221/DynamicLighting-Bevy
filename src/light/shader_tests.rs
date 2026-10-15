//! Compiles the light WESL modules with naga, so shader errors fail `cargo test`.

use crate::shared::wesl_check::{SHARED_TYPES, compile};

/// `(import path, source)` of the light-type WESL module, for other slices' shader tests.
pub(crate) const LIGHT_TYPES: (&str, &str) = (
    "dynlight::light::shaders::light_types",
    include_str!("shaders/light_types.wesl"),
);

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
        assert!(
            module
                .functions
                .iter()
                .any(|(_, function)| function.name.as_deref() == Some(name)),
            "{name} missing"
        );
    }
}
