//! The shadow cubemap array every lit material samples: variance moments `(d/r, (d/r)²)` per
//! texel, one cube (6 layers) per shadowed light. It grows with demand up to the budget.

use bevy::asset::{RenderAssetUsages, uuid_handle};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
    TextureViewDimension,
};
use bevy::render::settings::WgpuFeatures;

use super::faces::SHADOW_CUBEMAP_RESOLUTION;
use crate::shared::cube::CUBE_FACE_COUNT;

/// The shadow cubemap array `DynLitMaterial` binds.
pub const SHADOW_CUBEMAPS: Handle<Image> = uuid_handle!("b4d1e7a3-5c29-4f80-9e6b-2a7c3d8f1e05");

/// The formats a shadow cubemap array can have. Variance is `E[d²] - E[d]²`, which loses most
/// of its precision in 16-bit floats, so 32-bit floats (Henry's format) are used wherever the
/// device can filter them.
pub const SHADOW_CUBEMAP_FORMATS: [TextureFormat; 2] =
    [TextureFormat::Rg32Float, TextureFormat::Rg16Float];

/// The shadow cubemap format for a device with `features`.
pub fn shadow_cubemap_format(features: WgpuFeatures) -> TextureFormat {
    if features.contains(WgpuFeatures::FLOAT32_FILTERABLE) {
        TextureFormat::Rg32Float
    } else {
        TextureFormat::Rg16Float
    }
}

/// How many cubes the current array holds.
#[derive(Resource, Default, Debug)]
pub struct ShadowCubemapCapacity(pub u32);

/// An uninitialized array of `cubes` shadow cubemaps; with no cubes a 1×1 placeholder cube keeps
/// the binding valid. Only cubes rendered this frame are ever sampled.
pub fn cubemap_array(cubes: u32, format: TextureFormat) -> Image {
    let size = if cubes == 0 {
        1
    } else {
        SHADOW_CUBEMAP_RESOLUTION
    };
    let mut image = Image::new_uninit(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: cubes.max(1) * CUBE_FACE_COUNT,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_DST;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::CubeArray),
        ..default()
    });
    image.sampler = ImageSampler::linear();
    image
}

/// Grows the array to hold `needed` cubes. Capacity doubles so lights coming and going don't
/// reallocate it every frame; it never shrinks.
pub(super) fn ensure_capacity(
    needed: u32,
    capacity: &mut ShadowCubemapCapacity,
    images: &mut Assets<Image>,
    budget: u32,
    format: TextureFormat,
) {
    if needed <= capacity.0 {
        return;
    }
    let cubes = needed.next_power_of_two().min(budget.max(needed));
    if images
        .insert(&SHADOW_CUBEMAPS, cubemap_array(cubes, format))
        .is_ok()
    {
        capacity.0 = cubes;
    }
}

pub(super) fn create_placeholder(app: &mut App) {
    let Some(mut images) = app.world_mut().get_resource_mut::<Assets<Image>>() else {
        return;
    };
    images
        .insert(&SHADOW_CUBEMAPS, cubemap_array(0, TextureFormat::Rg16Float))
        .expect("the shadow cubemap handle is a fixed UUID");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrays_hold_six_layers_per_cube() {
        let image = cubemap_array(3, TextureFormat::Rg32Float);
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 18);
        assert_eq!(image.width(), SHADOW_CUBEMAP_RESOLUTION);
        assert!(image.data.is_none(), "rendered on the GPU, never uploaded");
        let placeholder = cubemap_array(0, TextureFormat::Rg16Float);
        assert_eq!(placeholder.texture_descriptor.size.depth_or_array_layers, 6);
    }

    #[test]
    fn capacity_doubles_up_to_the_budget() {
        let mut images = Assets::<Image>::default();
        let mut capacity = ShadowCubemapCapacity(0);
        ensure_capacity(3, &mut capacity, &mut images, 16, TextureFormat::Rg16Float);
        assert_eq!(capacity.0, 4);
        ensure_capacity(2, &mut capacity, &mut images, 16, TextureFormat::Rg16Float);
        assert_eq!(capacity.0, 4, "never shrinks");
        ensure_capacity(9, &mut capacity, &mut images, 12, TextureFormat::Rg16Float);
        assert_eq!(capacity.0, 12);
    }

    #[test]
    fn full_precision_moments_need_filterable_f32() {
        assert_eq!(
            shadow_cubemap_format(WgpuFeatures::FLOAT32_FILTERABLE),
            TextureFormat::Rg32Float
        );
        assert_eq!(
            shadow_cubemap_format(WgpuFeatures::empty()),
            TextureFormat::Rg16Float
        );
        assert!(SHADOW_CUBEMAP_FORMATS.contains(&shadow_cubemap_format(WgpuFeatures::empty())));
    }
}
