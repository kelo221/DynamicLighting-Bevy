//! Converts glTF scene content: `StandardMaterial` → `DynLitMaterial`, node extras → lights.

use bevy::gltf::GltfExtras;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::extras;
use crate::bake::BakeStatic;
use crate::surface::{DynLitMaterial, ShadingModel};

/// Settings for glTF conversion.
#[derive(Resource, Reflect, Clone, Debug, PartialEq)]
#[reflect(Resource, Default, Debug, Clone, PartialEq)]
pub struct GltfImportSettings {
    /// Replace every `StandardMaterial` that appears in the world (glTF or not) with an
    /// equivalent `DynLitMaterial`. Without Bevy lights, a `StandardMaterial` only shows
    /// ambient light.
    pub convert_standard_materials: bool,
    /// Create `DynamicLight`s and `BakeStatic` from `dynlight_*` node extras.
    pub read_node_extras: bool,
}

impl Default for GltfImportSettings {
    fn default() -> Self {
        Self {
            convert_standard_materials: true,
            read_node_extras: true,
        }
    }
}

/// The lit equivalent of a standard material: colors, textures, alpha and culling carry over.
/// Materials with metallic or roughness textures, normal maps or any metalness get
/// [`ShadingModel::Metallic`]; the rest get the cheaper Diffuse shading.
pub fn dynlit_from_standard(material: &StandardMaterial) -> DynLitMaterial {
    let metallic = material.metallic > 0.0
        || material.metallic_roughness_texture.is_some()
        || material.normal_map_texture.is_some();
    DynLitMaterial {
        base_color: material.base_color,
        base_color_texture: material.base_color_texture.clone(),
        emissive: material.emissive,
        emissive_texture: material.emissive_texture.clone(),
        shading: if metallic {
            ShadingModel::Metallic
        } else {
            ShadingModel::Diffuse
        },
        metallic: material.metallic,
        perceptual_roughness: material.perceptual_roughness,
        metallic_roughness_texture: material.metallic_roughness_texture.clone(),
        normal_map_texture: material.normal_map_texture.clone(),
        occlusion_texture: material.occlusion_texture.clone(),
        alpha_mode: material.alpha_mode,
        cull_mode: if material.double_sided {
            None
        } else {
            material.cull_mode
        },
        unlit: material.unlit,
        ..default()
    }
}

/// Conversion bookkeeping: entities whose standard material wasn't loaded yet when they
/// appeared, and the lit material made for each standard material.
#[derive(Resource, Default)]
pub(super) struct Conversions {
    pending: Vec<Entity>,
    converted: HashMap<AssetId<StandardMaterial>, Handle<DynLitMaterial>>,
}

pub(super) fn convert_materials(
    mut commands: Commands,
    settings: Res<GltfImportSettings>,
    added: Query<Entity, Added<MeshMaterial3d<StandardMaterial>>>,
    meshes: Query<&MeshMaterial3d<StandardMaterial>>,
    standard: Res<Assets<StandardMaterial>>,
    mut dynlit: ResMut<Assets<DynLitMaterial>>,
    mut conversions: ResMut<Conversions>,
) {
    if !settings.convert_standard_materials {
        return;
    }
    let conversions = &mut *conversions;
    let mut waiting = Vec::new();
    for entity in conversions.pending.drain(..).chain(added.iter()) {
        let Ok(handle) = meshes.get(entity) else {
            continue;
        };
        let id = handle.id();
        let lit = match conversions.converted.get(&id) {
            Some(lit) => lit.clone(),
            None => {
                let Some(material) = standard.get(id) else {
                    waiting.push(entity);
                    continue;
                };
                let lit = dynlit.add(dynlit_from_standard(material));
                conversions.converted.insert(id, lit.clone());
                lit
            }
        };
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(lit));
    }
    conversions.pending = waiting;
}

pub(super) fn read_node_extras(
    mut commands: Commands,
    settings: Res<GltfImportSettings>,
    nodes: Query<(Entity, &GltfExtras), Added<GltfExtras>>,
) {
    if !settings.read_node_extras {
        return;
    }
    for (entity, node) in &nodes {
        let parsed = extras::parse(&node.value);
        let mut entity = commands.entity(entity);
        if parsed.bake_static {
            entity.insert(BakeStatic);
        }
        if let Some(light) = parsed.light {
            entity.insert(light);
        }
        if let Some(effect) = parsed.effect {
            entity.insert(effect);
        }
        if let Some(fog) = parsed.fog {
            entity.insert(fog);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::render::render_resource::Face;

    use super::*;

    #[test]
    fn double_sided_materials_render_both_faces() {
        let standard = StandardMaterial {
            base_color: Color::srgb(0.5, 0.2, 0.1),
            double_sided: true,
            alpha_mode: AlphaMode::Mask(0.4),
            ..default()
        };
        let lit = dynlit_from_standard(&standard);
        assert_eq!(lit.cull_mode, None);
        assert_eq!(lit.base_color, standard.base_color);
        assert_eq!(lit.alpha_mode, AlphaMode::Mask(0.4));
        let single = dynlit_from_standard(&StandardMaterial::default());
        assert_eq!(single.cull_mode, Some(Face::Back));
    }
}
