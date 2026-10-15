//! Baked meshes with see-through [`DynLitMaterial`]s get a [`BakeAlphaMask`] from their base
//! color, so lights with transparency enabled shine through the clear parts at bake time.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

use crate::bake::{AlphaTexture, BakeAlphaMask, BakeStatic};

use super::material::DynLitMaterial;

/// Marks a [`BakeAlphaMask`] this slice derived from the material, so it can be replaced or
/// removed when the material changes. Masks added by hand are left alone.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct DerivedAlphaMask;

/// The alpha texture `material` bakes with: base color alpha times texture alpha, tested
/// against the mask cutoff (Henry's 0.5 for blended materials). `None` for opaque materials,
/// `Some(None)` while the texture isn't readable yet.
pub fn material_alpha(
    material: &DynLitMaterial,
    images: &Assets<Image>,
) -> Option<Option<AlphaTexture>> {
    let cutoff = match material.alpha_mode {
        AlphaMode::Mask(cutoff) => cutoff,
        AlphaMode::Blend | AlphaMode::Premultiplied | AlphaMode::AlphaToCoverage => {
            AlphaTexture::DEFAULT_CUTOFF
        }
        AlphaMode::Opaque | AlphaMode::Add | AlphaMode::Multiply => return None,
    };
    let factor = material.base_color.alpha();
    let Some(handle) = &material.base_color_texture else {
        let alpha = (factor.clamp(0.0, 1.0) * 255.0).round() as u8;
        return Some(Some(AlphaTexture {
            width: 1,
            height: 1,
            rgba8: vec![255, 255, 255, alpha],
            cutoff,
        }));
    };
    Some(images.get(handle).and_then(|image| {
        let mut rgba8 = rgba8(image)?;
        for texel in rgba8.as_chunks_mut::<4>().0 {
            texel[3] = (f32::from(texel[3]) * factor).round().clamp(0.0, 255.0) as u8;
        }
        Some(AlphaTexture {
            width: image.width(),
            height: image.height(),
            rgba8,
            cutoff,
        })
    }))
}

/// The image's texels as RGBA8, or `None` when they only exist on the GPU.
fn rgba8(image: &Image) -> Option<Vec<u8>> {
    match image.texture_descriptor.format {
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => image.data.clone(),
        _ => image
            .convert(TextureFormat::Rgba8Unorm)
            .and_then(|image| image.data)
            .or_else(|| {
                let (width, height) = (image.width(), image.height());
                let mut rgba8 = Vec::with_capacity((width * height * 4) as usize);
                for y in 0..height {
                    for x in 0..width {
                        let color = image.get_color_at(x, y).ok()?.to_srgba().to_u8_array();
                        rgba8.extend_from_slice(&color);
                    }
                }
                Some(rgba8)
            }),
    }
}

type MaskedMesh<'a> = (
    Entity,
    &'a MeshMaterial3d<DynLitMaterial>,
    Option<&'a BakeAlphaMask>,
    Has<DerivedAlphaMask>,
);
type MaskChanges = Or<(Added<BakeStatic>, Changed<MeshMaterial3d<DynLitMaterial>>)>;

/// Keeps derived [`BakeAlphaMask`]s in step with the materials of [`BakeStatic`] meshes.
pub(super) fn sync_alpha_masks(
    mut commands: Commands,
    mut material_events: MessageReader<AssetEvent<DynLitMaterial>>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    changed: Query<(), (MaskChanges, With<BakeStatic>)>,
    materials: Res<Assets<DynLitMaterial>>,
    images: Res<Assets<Image>>,
    meshes: Query<MaskedMesh, With<BakeStatic>>,
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
        let wanted = cache.entry(material.id()).or_insert_with(|| {
            materials
                .get(&material.0)
                .and_then(|material| material_alpha(material, &images))
        });
        match wanted {
            Some(Some(alpha)) => {
                if current.is_none_or(|mask| mask.0 != *alpha) {
                    commands
                        .entity(entity)
                        .insert((BakeAlphaMask(alpha.clone()), DerivedAlphaMask));
                }
            }
            // The texture is still loading; keep what we have until it arrives.
            Some(None) => {}
            None if derived => {
                commands
                    .entity(entity)
                    .remove::<(BakeAlphaMask, DerivedAlphaMask)>();
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_materials_have_no_mask() {
        let images = Assets::<Image>::default();
        let material = DynLitMaterial::from(Color::WHITE);
        assert_eq!(material_alpha(&material, &images), None);
    }

    #[test]
    fn untextured_masks_use_the_base_color_alpha() {
        let images = Assets::<Image>::default();
        let material = DynLitMaterial {
            alpha_mode: AlphaMode::Mask(0.3),
            base_color: Color::srgba(1.0, 1.0, 1.0, 0.2),
            ..default()
        };
        let alpha = material_alpha(&material, &images).unwrap().unwrap();
        assert_eq!(alpha.cutoff, 0.3);
        assert!(
            !alpha.is_opaque(Vec2::ZERO),
            "alpha 0.2 is below the cutoff"
        );
    }

    #[test]
    fn textures_are_scaled_by_the_base_color_alpha() {
        let mut images = Assets::<Image>::default();
        let image = Image::new_fill(
            bevy::render::render_resource::Extent3d {
                width: 2,
                height: 1,
                depth_or_array_layers: 1,
            },
            bevy::render::render_resource::TextureDimension::D2,
            &[255, 255, 255, 200],
            TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::all(),
        );
        let material = DynLitMaterial {
            alpha_mode: AlphaMode::Blend,
            base_color: Color::srgba(1.0, 1.0, 1.0, 0.5),
            base_color_texture: Some(images.add(image)),
            ..default()
        };
        let alpha = material_alpha(&material, &images).unwrap().unwrap();
        assert_eq!((alpha.width, alpha.height), (2, 1));
        assert_eq!(alpha.rgba8[3], 100);
        assert!(
            !alpha.is_opaque(Vec2::new(0.25, 0.5)),
            "100/255 is below 0.5"
        );
    }
}
