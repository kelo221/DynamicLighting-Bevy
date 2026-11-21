//! What the bake reads from a material: the color it reflects (bounce), the alpha mask it casts
//! shadows through, whether it blends, and the textures those come from. [`DynLitMaterial`]
//! answers from its own fields; an external material answers through [`ExternalBakeMaterial`].
//! The sync systems of `bake_albedo`, `alpha_mask` and `pending` are generic over this.

use bevy::pbr::Material;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;

use super::alpha_mask::{alpha_texture, blends, material_alpha};
use super::bake_albedo::{albedo_of, material_albedo};
use super::material::DynLitMaterial;
use crate::bake::AlphaTexture;

/// What an external material (`surface/external.rs`) tells DynLight so a [`BakeStatic`]
/// mesh that uses it bakes like a `DynLitMaterial`: coloured bounce light takes on its
/// albedo, and lights with transparency enabled shine through its clear parts. Implement it for
/// the material `M` of an [`ExternalLightingPlugin<M>`](super::ExternalLightingPlugin); every
/// method has a default, so an empty `impl ExternalBakeMaterial for M {}` bakes it as an opaque
/// white surface.
///
/// DynLight re-reads the answers when the material or one of the textures it returns changes,
/// and bakes wait until those textures are loaded.
///
/// [`BakeStatic`]: crate::bake::BakeStatic
pub trait ExternalBakeMaterial: Asset {
    /// The color the surface reflects: a linear color times the mean (linear) color of the
    /// texture, if any. Default: white, no texture.
    fn bake_albedo(&self) -> (LinearRgba, Option<Handle<Image>>) {
        (LinearRgba::WHITE, None)
    }

    /// The texture whose alpha channel makes the surface see-through, and the cutoff below
    /// which a texel is clear. `None` (the default): opaque.
    fn alpha_mask(&self) -> Option<(Handle<Image>, f32)> {
        None
    }

    /// Whether the material blends, so its meshes cast no shadows (realtime shadows skip them
    /// too). Default: false.
    fn blended(&self) -> bool {
        false
    }
}

/// The bake-relevant answers of a material, for the generic sync systems.
pub(super) trait BakeSurface: Material {
    /// The mean linear albedo; `None` while its texture isn't readable yet.
    fn albedo(&self, images: &Assets<Image>) -> Option<Vec3>;
    /// `None` for opaque materials, `Some(None)` while the texture isn't readable yet.
    fn alpha(&self, images: &Assets<Image>) -> Option<Option<AlphaTexture>>;
    /// Whether meshes with this material cast no shadows because they blend.
    fn blends(&self) -> bool;
    /// The textures the answers depend on.
    fn textures(&self) -> Vec<AssetId<Image>>;
}

impl BakeSurface for DynLitMaterial {
    fn albedo(&self, images: &Assets<Image>) -> Option<Vec3> {
        material_albedo(self, images)
    }

    fn alpha(&self, images: &Assets<Image>) -> Option<Option<AlphaTexture>> {
        material_alpha(self, images)
    }

    fn blends(&self) -> bool {
        blends(self.alpha_mode)
    }

    fn textures(&self) -> Vec<AssetId<Image>> {
        self.base_color_texture
            .iter()
            .map(|texture| texture.id())
            .collect()
    }
}

impl<M: Material + ExternalBakeMaterial> BakeSurface for M {
    fn albedo(&self, images: &Assets<Image>) -> Option<Vec3> {
        let (color, texture) = self.bake_albedo();
        albedo_of(
            Vec3::new(color.red, color.green, color.blue),
            texture.as_ref(),
            images,
        )
    }

    fn alpha(&self, images: &Assets<Image>) -> Option<Option<AlphaTexture>> {
        let (texture, cutoff) = self.alpha_mask()?;
        Some(alpha_texture(Some(&texture), 1.0, cutoff, images))
    }

    fn blends(&self) -> bool {
        self.blended()
    }

    fn textures(&self) -> Vec<AssetId<Image>> {
        let albedo = self.bake_albedo().1;
        let mask = self.alpha_mask().map(|(texture, _)| texture);
        albedo
            .iter()
            .chain(&mask)
            .map(|texture| texture.id())
            .collect()
    }
}

/// The materials an asset event batch can have changed: every material with an event of its own,
/// and every material one of whose textures has one. An image no material uses (a render target,
/// a fog map rewritten every few seconds) touches none.
pub(super) fn touched_materials<M: BakeSurface>(
    material_events: &mut MessageReader<AssetEvent<M>>,
    image_events: &mut MessageReader<AssetEvent<Image>>,
    materials: &Assets<M>,
) -> HashSet<AssetId<M>> {
    fn id<A: Asset>(event: &AssetEvent<A>) -> AssetId<A> {
        match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::Removed { id }
            | AssetEvent::Unused { id }
            | AssetEvent::LoadedWithDependencies { id } => *id,
        }
    }
    let mut touched: HashSet<_> = material_events.read().map(id).collect();
    let images: HashSet<_> = image_events.read().map(id).collect();
    if !images.is_empty() {
        touched.extend(materials.iter().filter_map(|(material_id, material)| {
            material
                .textures()
                .iter()
                .any(|texture| images.contains(texture))
                .then_some(material_id)
        }));
    }
    touched
}

#[cfg(test)]
mod tests {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};

    use super::*;
    use crate::bake::{BakeAlbedo, BakeAlphaMask, BakeStatic, BlendedSurface, SurfacePending};
    use crate::surface::{DerivedAlbedo, DerivedAlphaMask};

    #[derive(Asset, TypePath, AsBindGroup, Clone, Default)]
    struct Custom {
        #[uniform(0)]
        unused: f32,
        #[texture(1)]
        #[sampler(2)]
        bound: Option<Handle<Image>>,
        tint: Option<LinearRgba>,
        albedo_texture: Option<Handle<Image>>,
        mask: Option<(Handle<Image>, f32)>,
        glass: bool,
    }

    impl ExternalBakeMaterial for Custom {
        fn bake_albedo(&self) -> (LinearRgba, Option<Handle<Image>>) {
            (
                self.tint.unwrap_or(LinearRgba::WHITE),
                self.albedo_texture.clone(),
            )
        }
        fn alpha_mask(&self) -> Option<(Handle<Image>, f32)> {
            self.mask.clone()
        }
        fn blended(&self) -> bool {
            self.glass
        }
    }

    impl Material for Custom {}

    /// Relies on every default.
    #[derive(Asset, TypePath, AsBindGroup, Clone)]
    struct Plain {}
    impl Material for Plain {}
    impl ExternalBakeMaterial for Plain {}

    fn image(rgba: [u8; 4]) -> Image {
        Image::new_fill(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &rgba,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::all(),
        )
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Image>()
            .init_asset::<Custom>()
            .init_asset::<Plain>()
            .add_systems(
                Update,
                (
                    crate::surface::alpha_mask::sync_alpha_masks::<Custom>,
                    crate::surface::bake_albedo::sync_bake_albedos::<Custom>,
                    crate::surface::pending::sync_surface_pending::<Custom>,
                ),
            );
        app
    }

    #[test]
    fn defaults_are_an_opaque_white_surface() {
        let images = Assets::<Image>::default();
        assert_eq!(Plain {}.albedo(&images), Some(Vec3::ONE));
        assert_eq!(Plain {}.alpha(&images), None);
        assert!(!BakeSurface::blends(&Plain {}));
        assert!(Plain {}.textures().is_empty());
    }

    #[test]
    fn the_hook_feeds_the_albedo_and_the_alpha_mask() {
        let mut images = Assets::<Image>::default();
        let texture = images.add(image([255, 255, 255, 40]));
        let material = Custom {
            tint: Some(LinearRgba::new(0.5, 0.25, 1.0, 1.0)),
            mask: Some((texture.clone(), 0.5)),
            ..default()
        };
        assert_eq!(material.albedo(&images), Some(Vec3::new(0.5, 0.25, 1.0)));
        let mask = material.alpha(&images).unwrap().unwrap();
        assert_eq!(mask.cutoff, 0.5);
        assert!(!mask.is_opaque(Vec2::splat(0.5)), "alpha 40/255 is clear");
        assert_eq!(material.textures(), [texture.id()]);
    }

    #[test]
    fn unreadable_textures_wait() {
        let images = Assets::<Image>::default();
        let pending = images.reserve_handle();
        let material = Custom {
            albedo_texture: Some(pending.clone()),
            mask: Some((pending, 0.5)),
            ..default()
        };
        assert_eq!(material.albedo(&images), None);
        assert_eq!(material.alpha(&images), Some(None));
    }

    #[test]
    fn external_meshes_get_albedo_mask_blending_and_wait_for_textures() {
        let mut app = app();
        let texture = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .reserve_handle();
        let material = app
            .world_mut()
            .resource_mut::<Assets<Custom>>()
            .add(Custom {
                tint: Some(LinearRgba::new(0.2, 0.4, 0.6, 1.0)),
                mask: Some((texture.clone(), 0.5)),
                glass: true,
                ..default()
            });
        let mesh = app
            .world_mut()
            .spawn((BakeStatic, MeshMaterial3d(material.clone())))
            .id();
        app.update();
        let has = |app: &App, f: fn(&EntityRef) -> bool| f(&app.world().entity(mesh));
        assert!(
            has(&app, |e| e.contains::<SurfacePending>()),
            "texture loading"
        );
        assert!(has(&app, |e| e.contains::<BlendedSurface>()));
        assert!(has(&app, |e| e.contains::<BakeAlbedo>()
            && e.contains::<DerivedAlbedo>()));
        assert!(
            !has(&app, |e| e.contains::<BakeAlphaMask>()),
            "no texels yet"
        );

        let _ = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .insert(&texture, image([255, 255, 255, 0]));
        app.update();
        app.update();
        assert!(
            !has(&app, |e| e.contains::<SurfacePending>()),
            "texture arrived"
        );
        assert!(has(&app, |e| e.contains::<BakeAlphaMask>()
            && e.contains::<DerivedAlphaMask>()));
    }
}
