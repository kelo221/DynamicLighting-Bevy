//! Lighting for a material that is not a [`DynLitMaterial`](super::DynLitMaterial): the
//! realtime `DynamicLight`s lighting your own `Material`, for example an
//! `ExtendedMaterial` whose extension carries the bindings: realtime lights with their realtime
//! shadows, cookies and effects, and baked lighting (`BakeStatic` meshes bake like a
//! `DynLitMaterial`'s, see [`ExternalBakeMaterial`]).
//!
//! What a material needs to do:
//!
//! 1. Carry the scene-wide lighting resources at bindings 120–132 of its bind group
//!    ([`SCENE_BINDINGS`]). `#[derive(AsBindGroup)]` cannot nest a struct, so **copy the field
//!    block below into your material (or its extension) and fill the fields from
//!    [`DynLightBindings::default()`]**; [`DynLightBindings`] holds the same fields under the
//!    same names. The material's own bindings must stay out of 120–132 (a base material
//!    of an extended one uses the low numbers).
//! 2. In `Material::specialize` call [`external_specialize`] (it adds [`external_shader_defs`] to
//!    the fragment and, for meshes with triangle IDs, the vertex shader and attribute), and
//!    return [`external_vertex_shader`] from `Material::vertex_shader`.
//! 3. Add [`ExternalLightingPlugin<YourMaterial>`](ExternalLightingPlugin) after
//!    `DynamicLightingPlugin`, and `impl ExternalBakeMaterial for YourMaterial` (an empty impl
//!    bakes it as an opaque white surface).
//! 4. Call `surface_light` from the fragment shader (WESL). It sets `SurfaceInput` exactly as
//!    `dynlit.wesl` does: `lightmap_uv` is UV1 (`in.uv_b`, `VERTEX_UVS_B`) else UV0 (`in.uv`),
//!    which is what the bake used; `tag` and `primitive` find the mesh's baked data. The tag is
//!    the `MeshTag` DynLight gives a baked mesh (`get_tag(in.instance_index)`, 0 without bake
//!    data), the primitive `@builtin(primitive_index)` under `PRIMITIVE_INDEX`, or under
//!    `TRIANGLE_ID` the flat `@location(8)` input the vertex shader passes on (GPUs without
//!    primitive indices). With neither def the baked path is off: set both to 0.
//!
//! ```wesl
//! import bevy_pbr::render::mesh_functions::get_tag;
//! import dynlight::surface::shaders::light_processor::Shading;
//! import dynlight::surface::shaders::lighting::{SurfaceInput, surface_light};
//!
//! @if(PRIMITIVE_INDEX)
//! enable primitive_index;
//!
//! @fragment
//! fn fragment(
//!     in: VertexOutput,
//!     @builtin(front_facing) is_front: bool,
//!     @if(PRIMITIVE_INDEX) @builtin(primitive_index) primitive_index: u32,
//!     @if(TRIANGLE_ID) @location(8) @interpolate(flat) triangle_id: u32,
//! ) -> @location(0) vec4<f32> {
//!     var input: SurfaceInput;
//!     input.world = in.world_position.xyz;
//!     input.normal = normalize(in.world_normal); // flipped for back faces
//!     input.lightmap_uv = vec2(0.0);
//!     input.tag = 0u;       // no bake data: the dynamic path, lit by the realtime lights
//!     input.primitive = 0u;
//!     @if(VERTEX_UVS_B) { input.lightmap_uv = in.uv_b; }
//!     @elif(VERTEX_UVS_A) { input.lightmap_uv = in.uv; }
//!     @if(PRIMITIVE_INDEX) {
//!         input.tag = get_tag(in.instance_index);
//!         input.primitive = primitive_index;
//!     }
//!     @if(TRIANGLE_ID) {
//!         input.tag = get_tag(in.instance_index);
//!         input.primitive = triangle_id;
//!     }
//!     // view: unit vector to the camera; f0, roughness, metallic: your surface;
//!     // specular: false for Lambert diffuse only; sky_reflection: true.
//!     let lights = surface_light(input, Shading(view, vec3(0.04), 0.5, 0.0, false, true));
//!     let color = base * (ambient + lights.diffuse); // plus `lights.specular` with `specular`
//! }
//! ```
//!
//! Baked lighting needs `PRIMITIVE_INDEX` or the triangle-ID fallback; a material whose
//! `specialize` skips [`external_specialize`] and whose GPU has no primitive indices gets no
//! baked shadows, and DynLight warns about it once at startup (see [`ExternalLightingPlugin`]).
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
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline};
use bevy::prelude::*;
use bevy::render::render_resource::{RenderPipelineDescriptor, SpecializedMeshPipelineError};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::{ShaderDefVal, ShaderRef};

use super::alpha_mask::sync_alpha_masks;
use super::bake_albedo::sync_bake_albedos;
use super::bake_surface::ExternalBakeMaterial;
use super::material::VERTEX_SHADER;
use super::pending::sync_surface_pending;
use super::shared_arrays::rebind_shared_arrays;
use super::unbaked::note_unbaked_meshes;
use crate::baked_lighting::{
    ATTRIBUTE_TRIANGLE_ID, BAKED_AUX, BAKED_TRIANGLES, BakedReceiver, TRIANGLE_ID_LOCATION,
    primitive_index_missing, primitive_index_supported,
};
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
/// `@builtin(primitive_index)`, as for a `DynLitMaterial`. Prefer [`external_specialize`], which
/// also handles `TRIANGLE_ID`.
pub fn external_shader_defs() -> Vec<ShaderDefVal> {
    lit_shader_defs(false, false, true)
}

/// The vertex shader of an external material (`Material::vertex_shader`): DynLit's, which only
/// meshes with triangle IDs keep ([`external_specialize`] gives every other mesh Bevy's).
pub fn external_vertex_shader() -> ShaderRef {
    VERTEX_SHADER.into()
}

/// Everything an external material does in `Material::specialize` for DynLight: for meshes with
/// triangle IDs (the fallback of GPUs without `@builtin(primitive_index)`) the vertex attribute
/// and shader, and the fragment's [`external_shader_defs`] plus `TRIANGLE_ID`. Needs
/// [`external_vertex_shader`] as the material's vertex shader.
///
/// ```ignore
/// fn specialize(
///     pipeline: &MaterialPipeline,
///     descriptor: &mut RenderPipelineDescriptor,
///     layout: &MeshVertexBufferLayoutRef,
///     _: MaterialPipelineKey<Self>,
/// ) -> Result<(), SpecializedMeshPipelineError> {
///     dynlight::surface::external_specialize(pipeline, descriptor, layout)
/// }
/// ```
pub fn external_specialize(
    pipeline: &MaterialPipeline,
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
) -> Result<(), SpecializedMeshPipelineError> {
    let triangle_ids = use_triangle_ids(pipeline, descriptor, layout)?;
    if let Some(fragment) = descriptor.fragment.as_mut() {
        fragment
            .shader_defs
            .extend(lit_shader_defs(triangle_ids, false, true));
    }
    Ok(())
}

/// Whether the mesh draws with triangle IDs: adds the attribute to the vertex buffer then, and
/// otherwise gives the main pass Bevy's vertex shader. The prepass shares `specialize` but keeps
/// Bevy's prepass vertex shader.
pub(super) fn use_triangle_ids(
    pipeline: &MaterialPipeline,
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
) -> Result<bool, SpecializedMeshPipelineError> {
    let main_pass = descriptor
        .vertex
        .shader_defs
        .contains(&"MESH_PIPELINE".into());
    let triangle_ids = main_pass && layout.0.contains(ATTRIBUTE_TRIANGLE_ID);
    if triangle_ids {
        let ids = layout
            .0
            .get_layout(&[ATTRIBUTE_TRIANGLE_ID.at_shader_location(TRIANGLE_ID_LOCATION)])?;
        descriptor.vertex.buffers[0]
            .attributes
            .extend(ids.attributes);
    } else if main_pass {
        descriptor.vertex.shader = pipeline.mesh_pipeline.shader.clone();
    }
    Ok(triangle_ids)
}

/// Does for a material `M` what `SurfacePlugin` does for `DynLitMaterial`, so DynLight sees it:
/// reports its meshes (which have no bake data) for the `unbaked_meshes` shader feature that
/// `FeatureSelection::Automatic` compiles, marks its materials modified when a shared array
/// (shadow cubemaps, cookies, reflection cubes) is replaced so their bind groups are prepared
/// again, and bakes them: [`BakeStatic`](crate::bake::BakeStatic) meshes with an `M` get a
/// `BakeAlbedo` and `BakeAlphaMask` from [`ExternalBakeMaterial`], a `BlendedSurface` when `M`
/// blends, and a `SurfacePending` while the material's textures load. Triangles, UVs, shadow
/// bits and bounce need nothing else: the bake and the baked-lighting slice only look at
/// `Mesh3d`, `BakeStatic` and the mesh tag. Add it after `DynamicLightingPlugin`.
///
/// On a GPU without `@builtin(primitive_index)` baked meshes carry triangle IDs and draw with
/// [`external_vertex_shader`]; a material that doesn't call [`external_specialize`] then can't
/// find its baked data, and the plugin logs a warning at startup (once the device is known) if
/// a mesh with this material is baked.
///
/// Not done for external materials: a material's own reflection cube. For realtime shadows to
/// follow a caster whose material changes, also add `CasterMaterialPlugin::<M>`
/// (`M: CasterMaterial`).
pub struct ExternalLightingPlugin<M>(PhantomData<M>);

impl<M> Default for ExternalLightingPlugin<M> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<M: Material + ExternalBakeMaterial> Plugin for ExternalLightingPlugin<M> {
    fn build(&self, _: &mut App) {}

    fn finish(&self, app: &mut App) {
        let world = app.world();
        if world.contains_resource::<Assets<M>>() && world.contains_resource::<Assets<Image>>() {
            // Before the bake snapshots the scene in `PostUpdate`.
            app.add_systems(
                Update,
                (
                    sync_alpha_masks::<M>,
                    sync_bake_albedos::<M>,
                    sync_surface_pending::<M>,
                    rebind_shared_arrays::<M>,
                ),
            )
            .add_systems(
                PostUpdate,
                (note_unbaked_meshes::<M>, warn_without_triangle_ids::<M>),
            );
        }
    }
}

/// Once, when a baked mesh with an `M` draws on a GPU without primitive indices: such a mesh
/// only finds its baked data if `M::specialize` calls [`external_specialize`].
fn warn_without_triangle_ids<M: Material>(
    baked: Query<(), (With<MeshMaterial3d<M>>, With<BakedReceiver>)>,
    mut warned: Local<bool>,
) {
    if *warned || primitive_index_supported() || !primitive_index_missing() || baked.is_empty() {
        return;
    }
    *warned = true;
    warn!(
        "baked meshes use the external material {}, and this GPU has no `PRIMITIVE_INDEX`: \
         they carry triangle IDs, which only a material that calls `external_specialize` and \
         `external_vertex_shader` and reads `@location(8)` in its fragment can use (see \
         `dynlight::surface::external`); otherwise they are lit without baked shadows",
        core::any::type_name::<M>()
    );
}
