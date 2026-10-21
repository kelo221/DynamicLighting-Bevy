//! Converts `StandardMaterial`s to `DynLitMaterial`s.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use crate::surface::{DynLitMaterial, ShadingModel};

/// Settings for glTF conversion.
#[derive(Resource, Reflect, Clone, Debug, PartialEq)]
#[reflect(Resource, Default, Debug, Clone, PartialEq)]
pub struct GltfImportSettings {
    /// Replace every `StandardMaterial` that appears in the world (glTF or not) with an
    /// equivalent `DynLitMaterial`. Without Bevy lights, a `StandardMaterial` only shows
    /// ambient light.
    pub convert_standard_materials: bool,
    /// Create `DynamicLight`s, `BakeStatic`, `Room`s and `Portal`s from `dynlight_*` node
    /// extras.
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
/// appeared, and the lit material made for each standard material that still exists.
#[derive(Resource, Default)]
pub(super) struct Conversions {
    pending: Vec<Entity>,
    converted: HashMap<AssetId<StandardMaterial>, Handle<DynLitMaterial>>,
}

/// Keeps lit copies in step with their sources: a modified standard material rewrites its lit
/// material in place (meshes keep their handle), and a standard material that is gone is
/// forgotten so its lit material is freed once no mesh uses it.
pub(super) fn refresh_converted_materials(
    mut events: MessageReader<AssetEvent<StandardMaterial>>,
    standard: Res<Assets<StandardMaterial>>,
    mut dynlit: ResMut<Assets<DynLitMaterial>>,
    mut conversions: ResMut<Conversions>,
) {
    for event in events.read() {
        match *event {
            AssetEvent::Modified { id } => {
                let (Some(lit), Some(material)) =
                    (conversions.converted.get(&id), standard.get(id))
                else {
                    continue;
                };
                if let Err(error) = dynlit.insert(lit.id(), dynlit_from_standard(material)) {
                    warn!("dynlight: cannot refresh a converted material: {error}");
                }
            }
            AssetEvent::Unused { id } | AssetEvent::Removed { id } => {
                conversions.converted.remove(&id);
            }
            _ => {}
        }
    }
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

#[cfg(test)]
mod tests {
    use bevy::render::render_resource::Face;

    use super::*;
    use crate::gltf_import::GltfImportPlugin;

    /// A headless app with asset tracking and the glTF import plugin finished.
    fn conversion_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<StandardMaterial>()
            .init_asset::<DynLitMaterial>()
            .add_plugins(GltfImportPlugin);
        app.finish();
        app.cleanup();
        app
    }

    /// The lit material handle a mesh entity ended up with.
    fn lit_handle(app: &App, entity: Entity) -> Handle<DynLitMaterial> {
        app.world()
            .get::<MeshMaterial3d<DynLitMaterial>>(entity)
            .expect("the mesh was converted")
            .0
            .clone()
    }

    /// The base color of the lit material a mesh entity uses.
    fn lit_color(app: &App, entity: Entity) -> Color {
        let handle = lit_handle(app, entity);
        app.world()
            .resource::<Assets<DynLitMaterial>>()
            .get(&handle)
            .expect("the lit material exists")
            .base_color
    }

    /// Editing a converted standard material updates the lit material the mesh already uses,
    /// and dropping the standard material forgets the conversion.
    #[test]
    fn converted_materials_follow_their_source() {
        let mut app = conversion_app();
        let first_color = Color::srgb(0.8, 0.1, 0.1);
        let edited_color = Color::srgb(0.1, 0.2, 0.9);
        let source = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: first_color,
                ..default()
            });
        let mesh = app.world_mut().spawn(MeshMaterial3d(source.clone())).id();
        app.update();
        let converted = lit_handle(&app, mesh);
        assert_eq!(lit_color(&app, mesh), first_color);
        let mut standard = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
        standard
            .get_mut(&source)
            .expect("the source exists")
            .base_color = edited_color;
        app.update();
        assert_eq!(lit_handle(&app, mesh), converted);
        assert_eq!(lit_color(&app, mesh), edited_color);
        drop(source);
        for _ in 0..3 {
            app.update();
        }
        assert!(app.world().resource::<Conversions>().converted.is_empty());
        assert_eq!(lit_color(&app, mesh), edited_color);
    }

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
