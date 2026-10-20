//! `DynLitMaterial`: the lit surface material, Henry's Diffuse and Metallic shaders on Bevy's
//! mesh pipeline.

use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

use super::reflection::Reflection;
use crate::baked_lighting::{
    ATTRIBUTE_TRIANGLE_ID, BAKED_AUX, BAKED_TRIANGLES, TRIANGLE_ID_LOCATION,
    primitive_index_supported,
};
use crate::cookies::COOKIE_ARRAY;
use crate::light::{AREA_LIGHT_LUT, LIGHT_BUFFER};
use crate::light_clusters::REALTIME_CLUSTERS;
use crate::realtime_shadows::{CasterMaterial, SHADOW_CUBEMAPS, SUN_CASCADES};

const SHADER: &str = "embedded://dynlight/surface/shaders/dynlit.wesl";
const VERTEX_SHADER: &str = "embedded://dynlight/surface/shaders/dynlit_vertex.wesl";
const PREPASS_SHADER: &str = "embedded://dynlight/surface/shaders/dynlit_prepass.wesl";

/// How a [`DynLitMaterial`] reflects light.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShadingModel {
    /// Lambert diffuse only (Henry's Diffuse shader). The cheapest.
    #[default]
    Diffuse,
    /// Diffuse plus a GGX specular highlight per light, driven by `metallic` and
    /// `perceptual_roughness` (Henry's Metallic shader).
    Metallic,
}

/// A surface lit by `DynamicLight`s: the light types, effects, cookies and shimmer, baked
/// shadows and bounce light, and distance-cube shadows on moving meshes. Bevy's own lights
/// never affect it.
///
/// Texture conventions follow glTF: `metallic_roughness_texture` has roughness in green and
/// metallic in blue, `occlusion_texture` uses red, and `normal_map_texture` is a linear
/// tangent-space map that needs mesh tangents. `lights`, `cookies`, `baked_triangles`,
/// `baked_aux`, `shadow_cubemaps`, `area_light_lut`, `realtime_clusters` and `sun_cascades`
/// point at the scene-wide lighting resources and should stay at their defaults.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[uniform(0, DynLitUniform)]
#[bind_group_data(DynLitKey)]
pub struct DynLitMaterial {
    /// Multiplied with the texture and vertex colors. Alpha drives `alpha_mode`.
    pub base_color: Color,
    #[texture(1)]
    #[sampler(2)]
    pub base_color_texture: Option<Handle<Image>>,
    /// Light the surface gives off regardless of lighting, added after lighting.
    pub emissive: LinearRgba,
    #[texture(3)]
    #[sampler(4)]
    pub emissive_texture: Option<Handle<Image>>,
    pub shading: ShadingModel,
    /// 0 for dielectrics, 1 for metals. Metallic shading only.
    pub metallic: f32,
    /// 0 is mirror-like, 1 fully rough. Metallic shading only.
    pub perceptual_roughness: f32,
    #[texture(5)]
    #[sampler(6)]
    pub metallic_roughness_texture: Option<Handle<Image>>,
    #[texture(7)]
    #[sampler(8)]
    pub normal_map_texture: Option<Handle<Image>>,
    /// Scales the normal map's tangent-space slope.
    pub normal_scale: f32,
    #[texture(9)]
    #[sampler(10)]
    pub occlusion_texture: Option<Handle<Image>>,
    /// How much of the occlusion texture applies, 0 to 1.
    pub occlusion_strength: f32,
    /// What metallic surfaces reflect: the sky by default.
    #[texture(30, dimension = "cube_array")]
    #[sampler(31)]
    pub reflection: Reflection,
    /// `Mask(cutoff)` discards fragments below the cutoff; `Blend` is Henry's transparent mode.
    pub alpha_mode: AlphaMode,
    /// Faces to skip. `None` renders both sides and lights back faces with the flipped normal.
    pub cull_mode: Option<Face>,
    /// Shows the texture and color without any lighting.
    pub unlit: bool,
    #[storage(20, read_only)]
    pub lights: Handle<ShaderBuffer>,
    #[texture(21, dimension = "2d_array")]
    #[sampler(22)]
    pub cookies: Handle<Image>,
    #[storage(23, read_only)]
    pub baked_triangles: Handle<ShaderBuffer>,
    #[storage(24, read_only)]
    pub baked_aux: Handle<ShaderBuffer>,
    #[texture(25, dimension = "cube_array")]
    #[sampler(26)]
    pub shadow_cubemaps: Handle<Image>,
    #[texture(27, dimension = "2d_array")]
    #[sampler(28)]
    pub area_light_lut: Handle<Image>,
    #[storage(29, read_only)]
    pub realtime_clusters: Handle<ShaderBuffer>,
    /// Sampled with the shadow cubemaps' sampler.
    #[texture(32, dimension = "2d_array")]
    pub sun_cascades: Handle<Image>,
}

impl Default for DynLitMaterial {
    fn default() -> Self {
        Self {
            base_color: Color::WHITE,
            base_color_texture: None,
            emissive: LinearRgba::BLACK,
            emissive_texture: None,
            shading: ShadingModel::Diffuse,
            metallic: 0.0,
            perceptual_roughness: 0.5,
            metallic_roughness_texture: None,
            normal_map_texture: None,
            normal_scale: 1.0,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            reflection: Reflection::Sky,
            alpha_mode: AlphaMode::Opaque,
            cull_mode: Some(Face::Back),
            unlit: false,
            lights: LIGHT_BUFFER,
            cookies: COOKIE_ARRAY,
            baked_triangles: BAKED_TRIANGLES,
            baked_aux: BAKED_AUX,
            shadow_cubemaps: SHADOW_CUBEMAPS,
            area_light_lut: AREA_LIGHT_LUT,
            realtime_clusters: REALTIME_CLUSTERS,
            sun_cascades: SUN_CASCADES,
        }
    }
}

impl From<Color> for DynLitMaterial {
    fn from(base_color: Color) -> Self {
        Self {
            base_color,
            alpha_mode: if base_color.alpha() < 1.0 {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            },
            ..default()
        }
    }
}

impl From<Handle<Image>> for DynLitMaterial {
    fn from(texture: Handle<Image>) -> Self {
        Self {
            base_color_texture: Some(texture),
            ..default()
        }
    }
}

/// Flag bits in [`DynLitUniform::flags`]; `surface/shaders/bindings.wesl` mirrors them.
pub mod flags {
    pub const ALPHA_MASK: u32 = 1;
    pub const UNLIT: u32 = 2;
    pub const BASE_COLOR_TEXTURE: u32 = 4;
    pub const EMISSIVE_TEXTURE: u32 = 8;
    pub const METALLIC_ROUGHNESS_TEXTURE: u32 = 16;
    pub const NORMAL_MAP: u32 = 32;
    pub const OCCLUSION_TEXTURE: u32 = 64;
    pub const REFLECTION: u32 = 128;
    pub const METALLIC_SHADING: u32 = 256;
}

/// The material uniform at binding 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
pub struct DynLitUniform {
    /// Linear RGBA.
    pub base_color: Vec4,
    /// Linear RGB; alpha unused.
    pub emissive: Vec4,
    pub alpha_cutoff: f32,
    pub flags: u32,
    pub metallic: f32,
    pub perceptual_roughness: f32,
    pub normal_scale: f32,
    pub occlusion_strength: f32,
}

impl From<&DynLitMaterial> for DynLitUniform {
    fn from(material: &DynLitMaterial) -> Self {
        let mut bits = 0;
        let mut alpha_cutoff = 0.5;
        if let AlphaMode::Mask(cutoff) = material.alpha_mode {
            bits |= flags::ALPHA_MASK;
            alpha_cutoff = cutoff;
        }
        for (set, bit) in [
            (material.unlit, flags::UNLIT),
            (
                material.base_color_texture.is_some(),
                flags::BASE_COLOR_TEXTURE,
            ),
            (material.emissive_texture.is_some(), flags::EMISSIVE_TEXTURE),
            (
                material.metallic_roughness_texture.is_some(),
                flags::METALLIC_ROUGHNESS_TEXTURE,
            ),
            (material.normal_map_texture.is_some(), flags::NORMAL_MAP),
            (
                material.occlusion_texture.is_some(),
                flags::OCCLUSION_TEXTURE,
            ),
            (
                matches!(material.reflection, Reflection::Cubemap(_)),
                flags::REFLECTION,
            ),
            (
                material.shading == ShadingModel::Metallic,
                flags::METALLIC_SHADING,
            ),
        ] {
            if set {
                bits |= bit;
            }
        }
        Self {
            base_color: material.base_color.to_linear().to_vec4(),
            emissive: material.emissive.to_vec4(),
            alpha_cutoff,
            flags: bits,
            metallic: material.metallic.clamp(0.0, 1.0),
            perceptual_roughness: material.perceptual_roughness.clamp(0.0, 1.0),
            normal_scale: material.normal_scale,
            occlusion_strength: material.occlusion_strength.clamp(0.0, 1.0),
        }
    }
}

/// Pipeline variants of [`DynLitMaterial`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DynLitKey {
    pub cull_mode: Option<Face>,
    /// [`ShadingModel::Metallic`]: the shader's `METALLIC_SHADING` variant.
    pub metallic: bool,
}

impl From<&DynLitMaterial> for DynLitKey {
    fn from(material: &DynLitMaterial) -> Self {
        Self {
            cull_mode: material.cull_mode,
            metallic: material.shading == ShadingModel::Metallic,
        }
    }
}

impl Material for DynLitMaterial {
    /// Only meshes with triangle IDs keep this shader; `specialize` gives every other mesh
    /// Bevy's.
    fn vertex_shader() -> ShaderRef {
        VERTEX_SHADER.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        PREPASS_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = key.bind_group_data.cull_mode;
        // The prepass shares this function but keeps Bevy's prepass vertex shader.
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
        if let Some(fragment) = descriptor.fragment.as_mut() {
            if triangle_ids {
                fragment.shader_defs.push("TRIANGLE_ID".into());
            } else if primitive_index_supported() {
                fragment.shader_defs.push("PRIMITIVE_INDEX".into());
            }
            if key.bind_group_data.metallic {
                fragment.shader_defs.push("METALLIC_SHADING".into());
            }
        }
        Ok(())
    }
}

impl CasterMaterial for DynLitMaterial {
    fn visit_textures(&self, visit: &mut dyn FnMut(AssetId<Image>)) {
        [
            self.base_color_texture.as_ref(),
            self.emissive_texture.as_ref(),
            self.metallic_roughness_texture.as_ref(),
            self.normal_map_texture.as_ref(),
            self.occlusion_texture.as_ref(),
            self.reflection.cubemap(),
        ]
        .into_iter()
        .flatten()
        .for_each(|texture| visit(texture.id()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn casters_follow_the_materials_own_textures() {
        let texture = Handle::<Image>::default();
        let material = DynLitMaterial {
            normal_map_texture: Some(texture.clone()),
            ..DynLitMaterial::from(texture.clone())
        };
        let mut seen = Vec::new();
        material.visit_textures(&mut |id| seen.push(id));
        assert_eq!(seen, [texture.id(), texture.id()]);
    }

    #[test]
    fn only_a_cube_of_its_own_flags_the_reflection() {
        let flags = |reflection: Reflection| {
            let material = DynLitMaterial {
                reflection,
                ..default()
            };
            DynLitUniform::from(&material).flags & flags::REFLECTION
        };
        assert_eq!(flags(Reflection::Sky), 0);
        let cube = Reflection::Cubemap(Handle::default());
        assert_eq!(flags(cube), flags::REFLECTION);
    }

    #[test]
    fn uniform_carries_mask_cutoff_and_flags() {
        let material = DynLitMaterial {
            alpha_mode: AlphaMode::Mask(0.3),
            unlit: true,
            shading: ShadingModel::Metallic,
            metallic: 2.0,
            ..default()
        };
        let uniform = DynLitUniform::from(&material);
        assert_eq!(uniform.alpha_cutoff, 0.3);
        assert_eq!(
            uniform.flags,
            flags::ALPHA_MASK | flags::UNLIT | flags::METALLIC_SHADING
        );
        assert_eq!(uniform.metallic, 1.0);
    }

    #[test]
    fn translucent_colors_blend() {
        let material = DynLitMaterial::from(Color::srgba(1.0, 0.0, 0.0, 0.5));
        assert_eq!(material.alpha_mode, AlphaMode::Blend);
        assert_eq!(material.lights, LIGHT_BUFFER);
    }
}
