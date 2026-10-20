//! Compiles the reflection-probe WESL module with naga.

use crate::shared::wesl_check::compile;

pub(crate) const PROBES: (&str, &str) = (
    "dynlight::reflection_probes::shaders::probes",
    include_str!("shaders/probes.wesl"),
);

#[test]
fn probes_compile_and_validate() {
    let module = compile(PROBES.0, &[PROBES], &[]);
    for name in ["probe_box_point", "probe_holds", "probe_box_projected"] {
        let found = module
            .functions
            .iter()
            .any(|(_, function)| function.name.as_deref() == Some(name));
        assert!(found, "{name} is missing");
    }
}

#[test]
fn the_hold_margin_matches_the_mirror() {
    let line = format!(
        "const HOLD_MARGIN: f32 = {:?};",
        super::records::HOLD_MARGIN
    );
    assert!(PROBES.1.contains(&line), "probes.wesl lacks `{line}`");
}
