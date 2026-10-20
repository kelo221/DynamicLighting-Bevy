//! Baked meshes with a [`DynLitMaterial`] get a [`BakeAlbedo`]: the material's mean linear base
//! color, the color bounce light takes on where it leaves them (colored bounce only).

use bevy::platform::collections::HashMap;
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

/// Keeps derived [`BakeAlbedo`]s in step with the materials of [`BakeStatic`] meshes.
pub(super) fn sync_bake_albedos(
    mut commands: Commands,
    mut material_events: MessageReader<AssetEvent<DynLitMaterial>>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    changed: Query<(), (AlbedoChanges, With<BakeStatic>)>,
    materials: Res<Assets<DynLitMaterial>>,
    images: Res<Assets<Image>>,
    meshes: Query<AlbedoMesh, With<BakeStatic>>,
) {
    let assets_changed = material_events.read().count() > 0 || image_events.read().count() > 0;
    if !assets_changed && changed.is_empty() {
        return;
    }
    let mut cache = HashMap::new();
    for (entity, material, current, derived) in &meshes {
        if current.is_some() && !derived {
            continue;
        }
        let wanted = *cache.entry(material.id()).or_insert_with(|| {
            materials
                .get(&material.0)
                .and_then(|material| material_albedo(material, &images))
        });
        // Unknown until the material and its texture load; keep what we have.
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
}
