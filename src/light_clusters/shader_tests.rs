//! Compiles the cluster WESL module with naga and checks its constants against the Rust side.

use crate::shared::wesl_check::{MATERIAL_CONSTANTS, compile};

pub(crate) const CLUSTERS: (&str, &str) = (
    "dynlight::light_clusters::shaders::clusters",
    include_str!("shaders/clusters.wesl"),
);

pub(crate) fn compile_clusters() -> naga::Module {
    compile(CLUSTERS.0, &[MATERIAL_CONSTANTS, CLUSTERS], &[])
}

#[test]
fn clusters_compile_and_validate() {
    let module = compile_clusters();
    for name in ["realtime_span", "span_light"] {
        assert!(
            module
                .functions
                .iter()
                .any(|(_, function)| function.name.as_deref() == Some(name)),
            "{name} is missing"
        );
    }
}
