//! Builds the cookie texture array from the cookie images.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

/// Width and height of every cookie layer, as in Henry's package.
pub const COOKIE_RESOLUTION: u32 = 1024;
/// Most distinct cookie textures at once.
pub const COOKIE_BUDGET: usize = 64;

/// One cookie resampled to `resolution`², a linear red channel as 8-bit values. Returns `None`
/// when the image's pixels aren't readable on the CPU (rendering-only asset usage or a
/// compressed format).
pub fn resample(source: &Image, resolution: u32) -> Option<Vec<u8>> {
    let width = source.width();
    let height = source.height();
    if width == 0 || height == 0 {
        return None;
    }
    source.get_color_at(0, 0).ok()?;
    let texel = |x: i64, y: i64| {
        let x = x.clamp(0, width as i64 - 1) as u32;
        let y = y.clamp(0, height as i64 - 1) as u32;
        source
            .get_color_at(x, y)
            .map_or(0.0, |color| color.to_linear().red)
    };
    let mut layer = Vec::with_capacity((resolution * resolution) as usize);
    for y in 0..resolution {
        let v = (y as f32 + 0.5) / resolution as f32 * height as f32 - 0.5;
        let (y0, fy) = (v.floor(), v - v.floor());
        for x in 0..resolution {
            let u = (x as f32 + 0.5) / resolution as f32 * width as f32 - 0.5;
            let (x0, fx) = (u.floor(), u - u.floor());
            let (x0, y0) = (x0 as i64, y0 as i64);
            let top = texel(x0, y0) * (1.0 - fx) + texel(x0 + 1, y0) * fx;
            let bottom = texel(x0, y0 + 1) * (1.0 - fx) + texel(x0 + 1, y0 + 1) * fx;
            let value = top * (1.0 - fy) + bottom * fy;
            layer.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Some(layer)
}

/// The cookie array image: one `resolution`² R8 layer per entry, viewed as a 2D array and
/// sampled linearly with clamped edges. An empty list gives a single 1×1 white layer, so the
/// binding is always valid.
pub fn array_image(layers: &[Vec<u8>], resolution: u32) -> Image {
    let (size, data) = if layers.is_empty() {
        (1, vec![255])
    } else {
        (resolution, layers.concat())
    };
    let mut image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers.len().max(1) as u32,
        },
        TextureDimension::D2,
        data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = ImageSampler::linear();
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker() -> Image {
        let data = [
            0u8, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255,
        ];
        Image::new(
            Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data.to_vec(),
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::all(),
        )
    }

    #[test]
    fn resampling_keeps_the_pattern() {
        let layer = resample(&checker(), 8).unwrap();
        assert_eq!(layer.len(), 64);
        assert_eq!(layer[0], 0);
        assert_eq!(layer[7], 255);
        assert_eq!(layer[56], 255);
        assert_eq!(layer[63], 0);
    }

    #[test]
    fn empty_array_is_one_white_layer() {
        let image = array_image(&[], COOKIE_RESOLUTION);
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 1);
        assert_eq!(image.data.as_deref(), Some(&[255u8][..]));
    }

    #[test]
    fn layers_are_stacked() {
        let layers = vec![vec![1u8; 16], vec![2u8; 16]];
        let image = array_image(&layers, 4);
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 2);
        assert_eq!(image.data.as_ref().unwrap().len(), 32);
    }
}
