//! Baked meshes with a [`DynLitMaterial`] get a [`BakeAlbedo`]: the material's mean linear base
//! color, the color bounce light takes on where it leaves them (colored bounce only).

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

use crate::bake::{BakeAlbedo, BakeStatic};

use super::alpha_mask::rgba8;
use super::material::DynLitMaterial;

/// Marks a [`BakeAlbedo`] this slice derived from the material, so it follows the material.
/// Albedos added by hand are left alone.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct DerivedAlbedo;

/// The material's base color times the mean of its base color texture, linear. `None` while the
/// texture isn't readable yet.
pub fn material_albedo(material: &DynLitMaterial, images: &Assets<Image>) -> Option<Vec3> {
    let base = material.base_color.to_linear();
    let factor = Vec3::new(base.red, base.green, base.blue);
    let Some(handle) = &material.base_color_texture else {
        return Some(factor);
    };
    let image = images.get(handle)?;
    let srgb = image.texture_descriptor.format.is_srgb();
    let texels = rgba8(image)?;
    let decode: [f32; 256] = std::array::from_fn(|value| {
        let value = value as f32 / 255.0;
        if srgb {
            Color::srgb(value, 0.0, 0.0).to_linear().red
        } else {
            value
        }
    });
    let pixels = texels.as_chunks::<4>().0;
    let sum = pixels.iter().fold(Vec3::ZERO, |sum, texel| {
        sum + Vec3::new(
            decode[texel[0] as usize],
            decode[texel[1] as usize],
            decode[texel[2] as usize],
        )
    });
    Some(factor * sum / pixels.len().max(1) as f32)
}

type AlbedoMesh<'a> = (
    Entity,
    &'a MeshMaterial3d<DynLitMaterial>,
    Option<&'a BakeAlbedo>,
    Has<DerivedAlbedo>,
);
type AlbedoChanges = Or<(Added<BakeStatic>, Changed<MeshMaterial3d<DynLitMaterial>>)>;

/// The materials an asset event batch can have changed: every material with an event of its own, and every material whose base
/// color texture has one. An image no material uses (a render target, a fog map rewritten every few seconds) touches none.
pub(super) fn touched_materials(
    material_events: &mut MessageReader<AssetEvent<DynLitMaterial>>,
    image_events: &mut MessageReader<AssetEvent<Image>>,
    materials: &Assets<DynLitMaterial>,
) -> HashSet<AssetId<DynLitMaterial>> {
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
            let texture = material.base_color_texture.as_ref()?;
            images.contains(&texture.id()).then_some(material_id)
        }));
    }
    touched
}

/// Keeps derived [`BakeAlbedo`]s in step with the materials of [`BakeStatic`] meshes. A material's
/// albedo is worked out once and kept until that material or its texture changes.
#[allow(clippy::too_many_arguments)]
pub(super) fn sync_bake_albedos(
    mut commands: Commands,
    mut material_events: MessageReader<AssetEvent<DynLitMaterial>>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    changed: Query<(), (AlbedoChanges, With<BakeStatic>)>,
    materials: Res<Assets<DynLitMaterial>>,
    images: Res<Assets<Image>>,
    meshes: Query<AlbedoMesh, With<BakeStatic>>,
    mut known: Local<HashMap<AssetId<DynLitMaterial>, Vec3>>,
) {
    let touched = touched_materials(&mut material_events, &mut image_events, &materials);
    if touched.is_empty() && changed.is_empty() {
        return;
    }
    known.retain(|id, _| !touched.contains(id));
    for (entity, material, current, derived) in &meshes {
        if current.is_some() && !derived {
            continue;
        }
        let wanted = known.get(&material.id()).copied().or_else(|| {
            // Unknown until the material and its texture load; then it is kept.
            let albedo = material_albedo(materials.get(&material.0)?, &images)?;
            known.insert(material.id(), albedo);
            Some(albedo)
        });
        if let Some(albedo) = wanted
            && current.is_none_or(|current| current.0 != albedo)
        {
            commands
                .entity(entity)
                .insert((BakeAlbedo(albedo), DerivedAlbedo));
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

    use super::*;

    #[test]
    fn untextured_materials_reflect_their_linear_base_color() {
        let images = Assets::<Image>::default();
        let material = DynLitMaterial::from(Color::srgb(1.0, 0.5, 0.0));
        let albedo = material_albedo(&material, &images).unwrap();
        assert!(
            albedo.abs_diff_eq(Vec3::new(1.0, 0.2140, 0.0), 1e-3),
            "{albedo}"
        );
    }

    #[test]
    fn textures_are_averaged_in_linear_space() {
        let mut images = Assets::<Image>::default();
        let mut image = Image::new_fill(
            Extent3d {
                width: 2,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[255, 255, 255, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::all(),
        );
        if let Some(data) = image.data.as_mut() {
            data[4..8].copy_from_slice(&[0, 0, 0, 255]);
        }
        let material = DynLitMaterial {
            base_color: Color::linear_rgb(0.5, 1.0, 1.0),
            base_color_texture: Some(images.add(image)),
            ..default()
        };
        let albedo = material_albedo(&material, &images).unwrap();
        // White and black average to 0.5 in linear light, not to sRGB 0.5 (0.214).
        assert!(
            albedo.abs_diff_eq(Vec3::new(0.25, 0.5, 0.5), 1e-4),
            "{albedo}"
        );
    }

    #[test]
    fn unloaded_textures_wait() {
        let images = Assets::<Image>::default();
        let handle = images.reserve_handle();
        let material = DynLitMaterial {
            base_color_texture: Some(handle),
            ..default()
        };
        assert_eq!(material_albedo(&material, &images), None);
    }

    /// An image no material uses must not bring the albedos up for another look; a material's own texture must.
    #[test]
    fn only_a_materials_own_texture_touches_it() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Image>()
            .init_asset::<DynLitMaterial>();
        let pixel = || {
            Image::new_fill(
                Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[255, 255, 255, 255],
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::all(),
            )
        };
        let (used, unused) = {
            let mut images = app.world_mut().resource_mut::<Assets<Image>>();
            (images.add(pixel()), images.add(pixel()))
        };
        let material = app
            .world_mut()
            .resource_mut::<Assets<DynLitMaterial>>()
            .add(DynLitMaterial {
                base_color_texture: Some(used.clone()),
                ..default()
            });
        // One system with its own readers, as the sync systems have: (image events read, materials touched) of the last frame.
        #[derive(Resource, Default)]
        struct Seen(usize, HashSet<AssetId<DynLitMaterial>>);
        app.init_resource::<Seen>().add_systems(
            Last,
            |mut m: MessageReader<AssetEvent<DynLitMaterial>>,
             mut i: MessageReader<AssetEvent<Image>>,
             materials: Res<Assets<DynLitMaterial>>,
             mut last: ResMut<Seen>| {
                let seen = i.len();
                *last = Seen(seen, touched_materials(&mut m, &mut i, &materials));
            },
        );
        app.update(); // the assets' own `Added` events are read here
        let touched_by = |app: &mut App, image: &Handle<Image>| {
            if let Some(mut image) = app
                .world_mut()
                .resource_mut::<Assets<Image>>()
                .get_mut(image)
            {
                let _: &mut Image = &mut image; // the write is what marks it modified
            }
            app.update();
            let last = app.world().resource::<Seen>();
            (last.0, last.1.clone())
        };
        let (seen, touched) = touched_by(&mut app, &unused);
        assert!(seen > 0, "control: the unused image did send an event");
        assert!(
            !touched.contains(&material.id()),
            "an unrelated image touched the material"
        );
        let (_, touched) = touched_by(&mut app, &used);
        assert!(
            touched.contains(&material.id()),
            "the material's own texture did not touch it"
        );
    }
}
