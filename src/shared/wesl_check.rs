//! Test helpers: compile our WESL modules with the `wesl` version Bevy uses, then parse,
//! validate and inspect the resulting WGSL with naga.

use std::borrow::Cow;

use naga::valid::{Capabilities, ValidationFlags, Validator};
use wesl::CompileOptions;
use wesl::resolver::VirtualResolver;
use wesl::syntax::{ModulePath, PathOrigin};

/// `(import path, source)` of the shared WESL types module.
pub(crate) const SHARED_TYPES: (&str, &str) = (
    "dynlight::shared::shaders::types",
    include_str!("shaders/types.wesl"),
);

/// Bevy's virtual `constants` module as the material pipeline provides it.
pub(crate) const MATERIAL_CONSTANTS: (&str, &str) =
    ("constants", "const MATERIAL_BIND_GROUP = 3;\n");

/// A struct's size and member offsets in WGSL, in declaration order.
pub(crate) struct StructLayout {
    pub(crate) size: u32,
    pub(crate) members: Vec<(String, u32)>,
}

/// Resolves nested package names the way Bevy's shader resolver does: `dynlight/constants`
/// becomes `constants`, which is how `constants::X` reaches the virtual constants module.
struct BevyLikeResolver(VirtualResolver<'static>);

impl wesl::Resolver for BevyLikeResolver {
    fn resolve_source<'a>(
        &'a self,
        path: &ModulePath,
    ) -> Result<Cow<'a, str>, wesl::error::ResolveError> {
        self.0.resolve_source(&self.canonical_path(path))
    }

    fn canonical_path(&self, path: &ModulePath) -> ModulePath {
        match &path.origin {
            PathOrigin::Package(package) if package.contains('/') => ModulePath {
                origin: PathOrigin::Package(package.rsplit('/').next().unwrap().to_string()),
                components: path.components.clone(),
            },
            _ => path.clone(),
        }
    }
}

fn module_path(import_path: &str) -> ModulePath {
    let mut parts = import_path.split("::");
    let package = parts.next().expect("import path has a package");
    ModulePath {
        origin: PathOrigin::Package(package.to_string()),
        components: parts.map(str::to_string).collect(),
    }
}

/// Compiles the module at `main` against `modules`, with the given `@if` flags, and returns
/// the validated naga module. Panics with the compiler's message on any error.
pub(crate) fn compile(
    main: &str,
    modules: &[(&str, &str)],
    flags: &[(&str, bool)],
) -> naga::Module {
    let mut virtual_resolver = VirtualResolver::new();
    for (path, source) in modules {
        virtual_resolver.add_module(module_path(path), Cow::Owned(source.to_string()));
    }
    let resolver = BevyLikeResolver(virtual_resolver);
    let mut options = CompileOptions {
        imports: true,
        condcomp: true,
        visibility: false,
        strip: false,
        ..Default::default()
    };
    for (flag, enabled) in flags {
        options
            .features
            .flags
            .insert(flag.to_string(), (*enabled).into());
    }
    let wgsl = wesl::compile(&module_path(main), &options, &resolver)
        .unwrap_or_else(|error| panic!("WESL compile of {main} failed: {error}"))
        .to_string();
    let module = naga::front::wgsl::parse_str(&wgsl)
        .unwrap_or_else(|error| panic!("{main}: {}", error.emit_to_string(&wgsl)));
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|error| panic!("{main} failed validation: {error:?}"));
    module
}

/// `(import path, source)` of the generated shader variant module for `variant`.
pub(crate) fn shared_variant(
    variant: crate::shared::ShaderVariant,
) -> (&'static str, &'static str) {
    let source: &'static str = Box::leak(variant.source().into_boxed_str());
    ("dynlight::shared::shaders::variant", source)
}

pub(crate) fn compile_shared_types() -> naga::Module {
    compile(SHARED_TYPES.0, &[SHARED_TYPES], &[])
}

/// The WGSL layout of the struct named `name`. Panics if it doesn't exist.
pub(crate) fn struct_layout(module: &naga::Module, name: &str) -> StructLayout {
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .expect("layout of every type");
    let (handle, ty) = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no struct named {name}"));
    let naga::TypeInner::Struct { members, .. } = &ty.inner else {
        panic!("{name} is not a struct");
    };
    StructLayout {
        size: layouter[handle].size,
        members: members
            .iter()
            .map(|member| (member.name.clone().unwrap_or_default(), member.offset))
            .collect(),
    }
}

#[test]
fn shared_types_compile_and_validate() {
    let module = compile_shared_types();
    assert!(
        module
            .functions
            .iter()
            .any(|(_, function)| function.name.as_deref() == Some("light_type"))
    );
}
