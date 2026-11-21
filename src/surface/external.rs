//! Lighting for a material that is not a [`DynLitMaterial`](super::DynLitMaterial): the
//! realtime `DynamicLight`s lighting your own `Material`, for example an
//! `ExtendedMaterial` whose extension carries the bindings. Baking is not supported for such
//! materials; realtime lights are, with their realtime shadows, cookies and effects.
//!
//! What a material needs to do:
//!
//! 1. Carry the scene-wide lighting resources at bindings 120–132 of its bind group
//!    ([`SCENE_BINDINGS`]). `#[derive(AsBindGroup)]` cannot nest a struct, so **copy the field
//!    block below into your material (or its extension) and fill the fields from
//!    [`DynLightBindings::default()`]**; [`DynLightBindings`] holds the same fields under the
//!    same names. The material's own bindings must stay out of 120–132 (a base material
//!    of an extended one uses the low numbers).
//! 2. Add [`external_shader_defs`] to the fragment's shader defs in `Material::specialize`.
//! 3. Add [`ExternalLightingPlugin<YourMaterial>`](ExternalLightingPlugin) after
//!    `DynamicLightingPlugin`.
//! 4. Call `surface_light` from the fragment shader (WESL):
//!
//! ```wesl
//! import dynlight::surface::shaders::light_processor::Shading;
//! import dynlight::surface::shaders::lighting::{SurfaceInput, surface_light};
//!
//! var input: SurfaceInput;
//! input.world = in.world_position.xyz;
//! input.normal = normalize(in.world_normal); // flipped for back faces
//! input.lightmap_uv = vec2(0.0);
//! input.tag = 0u;       // no bake data: the dynamic path, lit by the realtime lights
//! input.primitive = 0u;
//! // view: unit vector to the camera; f0, roughness, metallic: your surface;
//! // specular: false for Lambert diffuse only; sky_reflection: true.
//! let lights = surface_light(input, Shading(view, vec3(0.04), 0.5, 0.0, false, true));
//! let color = base * (ambient + lights.diffuse); // plus `lights.specular` with `specular`
//! ```
//!
//! The field block (the doc of [`DynLightBindings`] explains each field):
//!
//! ```ignore
//! #[storage(120, read_only)]
//! pub dynlight_lights: Handle<ShaderBuffer>,
//! #[texture(121, dimension = "2d_array")]
//! #[sampler(122)]
//! pub dynlight_cookies: Handle<Image>,
//! #[storage(123, read_only)]
//! pub dynlight_baked_triangles: Handle<ShaderBuffer>,
//! #[storage(124, read_only)]
//! pub dynlight_baked_aux: Handle<ShaderBuffer>,
//! #[texture(125, dimension = "cube_array")]
//! #[sampler(126)]
//! pub dynlight_shadow_cubemaps: Handle<Image>,
//! #[texture(127, dimension = "2d_array")]
//! #[sampler(128)]
//! pub dynlight_area_light_lut: Handle<Image>,
//! #[storage(129, read_only)]
//! pub dynlight_realtime_clusters: Handle<ShaderBuffer>,
//! #[texture(130, dimension = "cube_array")]
//! #[sampler(131)]
//! pub dynlight_reflection: Handle<Image>,
//! // Sampled with the shadow cubemaps' sampler (126).
//! #[texture(132, dimension = "2d_array")]
//! pub dynlight_sun_cascades: Handle<Image>,
//! ```
//!
//! `examples/external_material.rs` is a complete material.

use core::marker::PhantomData;

use bevy::asset::Handle;
use bevy::pbr::Material;
use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderDefVal;

use super::shared_arrays::rebind_shared_arrays;
use super::unbaked::note_unbaked_meshes;
use crate::baked_lighting::{BAKED_AUX, BAKED_TRIANGLES, primitive_index_supported};
use crate::cookies::COOKIE_ARRAY;
use crate::light::{AREA_LIGHT_LUT, LIGHT_BUFFER};
use crate::light_clusters::REALTIME_CLUSTERS;
use crate::realtime_shadows::{SHADOW_CUBEMAPS, SUN_CASCADES};
use crate::reflection_probes::REFLECTION_CUBES;

/// The first scene-wide binding of an external material; `DynLitMaterial`'s is 20.
pub const SCENE_BINDING_BASE: u32 = 120;

/// The scene-wide bindings under `DYNLIGHT_EXTERNAL`: the WESL name of each declaration and
/// its binding in the material bind group. The fields of [`DynLightBindings`] follow this order
/// (a texture's sampler is the next binding).
pub const SCENE_BINDINGS: [(&str, u32); 13] = [
    ("dynamic_lights", 120),
    ("light_cookies", 121),
    ("light_cookie_sampler", 122),
    ("baked_triangles", 123),
    ("baked_aux", 124),
    ("shadow_cubemaps", 125),
    ("shadow_sampler", 126),
    ("area_light_lut", 127),
    ("area_light_lut_sampler", 128),
    ("realtime_clusters", 129),
    ("reflection_texture", 130),
    ("reflection_sampler", 131),
    ("sun_cascades", 132),
];

/// The scene-wide lighting resources of an external material, at their defaults: the global
/// handles. Copy the field block from the module header into your material's `AsBindGroup`
/// struct and fill it from this (`let bindings = DynLightBindings::default();`); the fields
/// point at resources shared by the whole scene and should stay at their defaults. Each field
/// is bound as the module header shows, at the binding of [`SCENE_BINDINGS`].
#[derive(Clone, Debug)]
pub struct DynLightBindings {
    /// `LIGHT_BUFFER`: every light, the globals, the sky and the probes. Binding 120.
    pub lights: Handle<ShaderBuffer>,
    /// `COOKIE_ARRAY`, a 2D array. Bindings 121 and 122 (sampler).
    pub cookies: Handle<Image>,
    /// `BAKED_TRIANGLES`. Binding 123.
    pub baked_triangles: Handle<ShaderBuffer>,
    /// `BAKED_AUX`. Binding 124.
    pub baked_aux: Handle<ShaderBuffer>,
    /// `SHADOW_CUBEMAPS`, a cube array. Bindings 125 and 126 (sampler).
    pub shadow_cubemaps: Handle<Image>,
    /// `AREA_LIGHT_LUT`, a 2D array. Bindings 127 and 128 (sampler).
    pub area_light_lut: Handle<Image>,
    /// `REALTIME_CLUSTERS`. Binding 129.
    pub realtime_clusters: Handle<ShaderBuffer>,
    /// The sky's and the probes' reflection cube (`REFLECTION_CUBES`), a cube array. Bindings
    /// 130 and 131 (sampler).
    pub reflection: Handle<Image>,
    /// `SUN_CASCADES`, a 2D array, sampled with the shadow cubemaps' sampler. Binding 132.
    pub sun_cascades: Handle<Image>,
}

impl Default for DynLightBindings {
    fn default() -> Self {
        Self {
            lights: LIGHT_BUFFER,
            cookies: COOKIE_ARRAY,
            baked_triangles: BAKED_TRIANGLES,
            baked_aux: BAKED_AUX,
            shadow_cubemaps: SHADOW_CUBEMAPS,
            area_light_lut: AREA_LIGHT_LUT,
            realtime_clusters: REALTIME_CLUSTERS,
            reflection: REFLECTION_CUBES,
            sun_cascades: SUN_CASCADES,
        }
    }
}

/// The fragment shader defs of a lit material, shared by `DynLitMaterial::specialize` and
/// [`external_shader_defs`] so they cannot drift. The quality and the shader features are not
/// defs: they are constants of the generated `dynlight::shared::shaders::variant` module that
/// the lighting imports.
pub(super) fn lit_shader_defs(
    triangle_ids: bool,
    metallic: bool,
    external: bool,
) -> Vec<ShaderDefVal> {
    let mut defs = Vec::new();
    if triangle_ids {
        defs.push("TRIANGLE_ID".into());
    } else if primitive_index_supported() {
        defs.push("PRIMITIVE_INDEX".into());
    }
    if metallic {
        defs.push("METALLIC_SHADING".into());
    }
    if external {
        defs.push("DYNLIGHT_EXTERNAL".into());
    }
    defs
}

/// The shader defs an external material adds to its fragment (`descriptor.fragment`, in
/// `Material::specialize`): `DYNLIGHT_EXTERNAL`, which moves the scene-wide bindings to 120–132
/// and drops `DynLitMaterial`'s own, and `PRIMITIVE_INDEX` where the GPU supports
/// `@builtin(primitive_index)`, as for a `DynLitMaterial` (only your own fragment can use it:
/// the lighting needs it for baked meshes, which external materials don't support).
///
/// ```ignore
/// if let Some(fragment) = descriptor.fragment.as_mut() {
///     fragment.shader_defs.extend(dynlight::surface::external_shader_defs());
/// }
/// ```
pub fn external_shader_defs() -> Vec<ShaderDefVal> {
    lit_shader_defs(false, false, true)
}

/// Does for a material `M` what `SurfacePlugin` does for `DynLitMaterial`, so DynLight sees it:
/// reports its meshes (which have no bake data) for the `unbaked_meshes` shader feature that
/// `FeatureSelection::Automatic` compiles, and marks its materials modified when a shared
/// array (shadow cubemaps, cookies, reflection cubes) is replaced so their bind groups are
/// prepared again. Add it after `DynamicLightingPlugin`.
///
/// Not done for external materials: baking (`BakeStatic`, albedo and alpha-mask derivation) and
/// a material's own reflection cube. For realtime shadows to follow a caster whose material
/// changes, also add `CasterMaterialPlugin::<M>` (`M: CasterMaterial`).
pub struct ExternalLightingPlugin<M>(PhantomData<M>);

impl<M> Default for ExternalLightingPlugin<M> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<M: Material> Plugin for ExternalLightingPlugin<M> {
    fn build(&self, _: &mut App) {}

    fn finish(&self, app: &mut App) {
        let world = app.world();
        if world.contains_resource::<Assets<M>>() && world.contains_resource::<Assets<Image>>() {
            app.add_systems(Update, rebind_shared_arrays::<M>)
                .add_systems(PostUpdate, note_unbaked_meshes::<M>);
        }
    }
}
