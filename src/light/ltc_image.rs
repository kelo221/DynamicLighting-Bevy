//! The LTC table (`ltc`) as the texture every lit material binds for rectangle-light specular.

use bevy::asset::{RenderAssetUsages, uuid_handle};
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

use super::ltc::{LUT_BYTES, LUT_SIZE};

/// The LTC table bound by `DynLitMaterial`: two RGBA16F layers viewed as a 2D array.
pub const AREA_LIGHT_LUT: Handle<Image> = uuid_handle!("bb2b402a-c8d0-4bb2-87fd-914a506e6dc1");

/// The table image, sampled linearly with clamped edges like `ltc::sample_layer`.
pub fn lut_image() -> Image {
    let mut image = Image::new(
        Extent3d {
            width: LUT_SIZE,
            height: LUT_SIZE,
            depth_or_array_layers: 2,
        },
        TextureDimension::D2,
        LUT_BYTES.to_vec(),
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = ImageSampler::linear();
    image
}

/// Adds the table under [`AREA_LIGHT_LUT`] in apps with images.
pub(super) fn insert_lut(app: &mut App) {
    if let Some(mut images) = app.world_mut().get_resource_mut::<Assets<Image>>() {
        images
            .insert(&AREA_LIGHT_LUT, lut_image())
            .expect("the LTC table handle is a fixed UUID");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_a_two_layer_array() {
        let image = lut_image();
        let size = image.texture_descriptor.size;
        assert_eq!(
            (size.width, size.height, size.depth_or_array_layers),
            (64, 64, 2)
        );
        assert_eq!(image.data.as_ref().map(Vec::len), Some(LUT_BYTES.len()));
        let view = image
            .texture_view_descriptor
            .as_ref()
            .and_then(|view| view.dimension);
        assert_eq!(view, Some(TextureViewDimension::D2Array));
    }
}
