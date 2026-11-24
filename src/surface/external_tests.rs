//! The external-material hook: the lighting WESL compiles with and without `DYNLIGHT_EXTERNAL`,
//! the bindings sit where `SCENE_BINDINGS` and the example's field block say, and `DynLitMaterial`
//! is unchanged without the def.

use std::collections::BTreeSet;

use super::external::{SCENE_BINDING_BASE, SCENE_BINDINGS, external_shader_defs, lit_shader_defs};
use super::shader_tests::{LIGHT_PROCESSOR, LIGHTING, SKY_LIGHT, surface_modules};
use crate::baked_lighting::shader_tests::{BAKED_DATA, TRIANGLES};
use crate::light_clusters::shader_tests::CLUSTERS;
use crate::realtime_shadows::shader_tests::{SUN_VSM, VSM};
use crate::shared::wesl_check::compile;
use crate::shared::{ShaderFeatures, ShaderVariant};

/// `(name, group, binding)` of every resource the module declares.
fn bindings(module: &naga::Module) -> Vec<(String, u32, u32)> {
    module
        .global_variables
        .iter()
        .filter_map(|(_, var)| {
            let binding = var.binding.as_ref()?;
            Some((
                var.name.clone().unwrap_or_default(),
                binding.group,
                binding.binding,
            ))
        })
        .collect()
}

const MATERIAL_GROUP: u32 = 3;

#[test]
fn lighting_modules_compile_with_and_without_the_def() {
    for external in [false, true] {
        for main in [
            LIGHT_PROCESSOR,
            SKY_LIGHT,
            LIGHTING,
            BAKED_DATA,
            CLUSTERS,
            VSM,
            SUN_VSM,
        ] {
            compile(
                main.0,
                &surface_modules(),
                &[("DYNLIGHT_EXTERNAL", external)],
            );
        }
    }
}

#[test]
fn lighting_compiles_under_the_def_in_every_variant() {
    for features in [ShaderFeatures::ALL, ShaderFeatures::NONE] {
        let mut modules = surface_modules();
        modules[2] = crate::shared::wesl_check::shared_variant(ShaderVariant {
            features,
            ..ShaderVariant::default()
        });
        compile(LIGHTING.0, &modules, &[("DYNLIGHT_EXTERNAL", true)]);
    }
}

#[test]
fn the_def_moves_every_scene_binding_to_120_132() {
    let module = compile(
        LIGHTING.0,
        &surface_modules(),
        &[("DYNLIGHT_EXTERNAL", true)],
    );
    let found = bindings(&module);
    let numbers: BTreeSet<u32> = found.iter().map(|(_, _, binding)| *binding).collect();
    let expected: BTreeSet<u32> = SCENE_BINDINGS.iter().map(|(_, binding)| *binding).collect();
    assert_eq!(numbers, expected);
    assert_eq!(
        (SCENE_BINDING_BASE..SCENE_BINDING_BASE + 13).collect::<BTreeSet<_>>(),
        expected
    );
    for (name, group, binding) in &found {
        assert_eq!(*group, MATERIAL_GROUP, "{name}");
        // Mangled names end in the declaration's own name.
        let declared = SCENE_BINDINGS
            .iter()
            .find(|(own, _)| name.ends_with(own))
            .unwrap_or_else(|| panic!("{name} is not in SCENE_BINDINGS"));
        assert_eq!(declared.1, *binding, "{name}");
    }
}

#[test]
fn without_the_def_the_bindings_stay_at_20_32() {
    let module = compile(LIGHTING.0, &surface_modules(), &[]);
    // The material's own bindings, 0–10, are DynLit's.
    let scene: BTreeSet<u32> = bindings(&module)
        .into_iter()
        .map(|(_, _, binding)| binding)
        .filter(|binding| *binding >= 20)
        .collect();
    assert_eq!(scene, (20..33).collect::<BTreeSet<_>>());
}

#[test]
fn the_lighting_path_does_not_need_the_material_uniform_under_the_def() {
    let module = compile(
        LIGHTING.0,
        &surface_modules(),
        &[("DYNLIGHT_EXTERNAL", true)],
    );
    assert!(
        bindings(&module)
            .iter()
            .all(|(_, _, binding)| *binding >= 100),
        "a DynLitMaterial binding is declared"
    );
}

/// The binding numbers of the attributes in the example's field block are the ones of
/// `SCENE_BINDINGS`; the sampler of a texture is the next number.
#[test]
fn the_example_material_binds_what_the_wesl_declares() {
    let source = include_str!("../../examples/external_material.rs");
    let mut found = BTreeSet::new();
    for attribute in ["storage(", "texture(", "sampler("] {
        for part in source.split(&format!("#[{attribute}")).skip(1) {
            let number: String = part.chars().take_while(char::is_ascii_digit).collect();
            let number: u32 = number.parse().expect("a binding number");
            if number >= SCENE_BINDING_BASE {
                found.insert(number);
            }
        }
    }
    let expected: BTreeSet<u32> = SCENE_BINDINGS.iter().map(|(_, binding)| *binding).collect();
    assert_eq!(found, expected);
}

#[test]
fn external_defs_extend_the_dynlit_defs() {
    let has = |defs: &[bevy::shader::ShaderDefVal], name: &'static str| defs.contains(&name.into());
    let dynlit = lit_shader_defs(true, true, false);
    assert!(has(&dynlit, "TRIANGLE_ID") && has(&dynlit, "METALLIC_SHADING"));
    assert!(!has(&dynlit, "DYNLIGHT_EXTERNAL"));
    let external = external_shader_defs();
    assert!(has(&external, "DYNLIGHT_EXTERNAL"));
    // The same list as DynLit's without the def (no GPU in tests, so no primitive index).
    let mut without = external.clone();
    without.retain(|def| *def != "DYNLIGHT_EXTERNAL".into());
    assert_eq!(without, lit_shader_defs(false, false, false));
}

/// The baked path under the def: the triangle and aux buffers the fragment reads by tag and
/// primitive are the ones at 123 and 124.
#[test]
fn the_baked_path_reads_buffers_123_and_124_under_the_def() {
    for main in [BAKED_DATA, TRIANGLES] {
        let module = compile(main.0, &surface_modules(), &[("DYNLIGHT_EXTERNAL", true)]);
        for (name, group, binding) in bindings(&module) {
            assert_eq!(group, MATERIAL_GROUP, "{name}");
            let expected = SCENE_BINDINGS
                .iter()
                .find(|(own, _)| name.ends_with(own))
                .unwrap_or_else(|| panic!("{name} is not in SCENE_BINDINGS"));
            assert_eq!(expected.1, binding, "{name}");
        }
    }
    let module = compile(
        TRIANGLES.0,
        &surface_modules(),
        &[("DYNLIGHT_EXTERNAL", true)],
    );
    let names: Vec<_> = bindings(&module).into_iter().map(|(n, ..)| n).collect();
    assert!(
        names.iter().any(|n| n.ends_with("baked_triangles")),
        "{names:?}"
    );
    assert!(names.iter().any(|n| n.ends_with("baked_aux")), "{names:?}");
}

/// The example's fragment sets `SurfaceInput` as `dynlit.wesl` does and as `external.rs`
/// documents: lightmap UV1 else UV0, the mesh tag, and the primitive from the builtin or the
/// triangle-ID input.
#[test]
fn the_example_fragment_sets_the_baked_inputs_like_dynlit() {
    let example = include_str!("../../examples/external_material.wesl");
    let dynlit = include_str!("shaders/dynlit.wesl");
    for line in [
        "input.lightmap_uv = in.uv_b;",
        "input.lightmap_uv = in.uv;",
        "input.tag = get_tag(in.instance_index);",
        "input.primitive = primitive_index;",
        "input.primitive = triangle_id;",
        "@if(PRIMITIVE_INDEX) @builtin(primitive_index) primitive_index: u32,",
        "@if(TRIANGLE_ID) @location(8) @interpolate(flat) triangle_id: u32,",
        "enable primitive_index;",
    ] {
        assert!(dynlit.contains(line), "dynlit.wesl lost `{line}`");
        assert!(
            example.contains(line),
            "the example's fragment lacks `{line}`"
        );
    }
    assert!(
        example.find("input.lightmap_uv = in.uv_b;") < example.find("input.lightmap_uv = in.uv;"),
        "UV1 is preferred"
    );
}
