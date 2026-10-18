//! Conversions between Bevy images and the sky's CPU data: reading the source image, and the
//! specular cube and background images every frame binds.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageSampler, ImageSamplerDescriptor, TextureFormatPixelInfo};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use half::f16;

use super::equirect::{Equirect, Pyramid};
use super::prefilter::SpecularCube;
use crate::shared::cube::CUBE_FACE_COUNT;

/// The largest radiance an `Rgba16Float` texel holds.
const F16_MAX: f32 = 65_000.0;

/// Why a sky image can't be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkyImageError {
    /// The image lives only on the GPU (`RenderAssetUsages::RENDER_WORLD`).
    NoCpuData,
    /// Not a 2D image with one layer.
    NotFlat,
    /// A format other than RGBA 32-bit float, 16-bit float or 8-bit.
    Format(TextureFormat),
}

impl core::fmt::Display for SkyImageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoCpuData => write!(f, "the image has no CPU data; keep MAIN_WORLD usage"),
            Self::NotFlat => write!(f, "the image must be one 2D equirectangular layer"),
            Self::Format(format) => write!(f, "unsupported format {format:?}"),
        }
    }
}

/// The linear RGB radiance of an equirectangular image's first mip level. Float images are
/// taken as linear; 8-bit sRGB images are converted.
pub fn equirect_from_image(image: &Image) -> Result<Equirect, SkyImageError> {
    let descriptor = &image.texture_descriptor;
    if descriptor.dimension != TextureDimension::D2 || descriptor.size.depth_or_array_layers != 1 {
        return Err(SkyImageError::NotFlat);
    }
    let data = image.data.as_deref().ok_or(SkyImageError::NoCpuData)?;
    let (width, height) = (descriptor.size.width, descriptor.size.height);
    let format = descriptor.format;
    let pixel = format
        .pixel_size()
        .map_err(|_| SkyImageError::Format(format))?;
    let count = (width * height) as usize;
    if width == 0 || height == 0 || data.len() < count * pixel {
        return Err(SkyImageError::NoCpuData);
    }
    let channel = |bytes: &[u8], i: usize| -> f32 {
        match format {
            TextureFormat::Rgba32Float => {
                f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap_or_default())
            }
            TextureFormat::Rgba16Float => {
                f16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]).to_f32()
            }
            TextureFormat::Rgba8UnormSrgb => Srgba::gamma_function(f32::from(bytes[i]) / 255.0),
            _ => f32::from(bytes[i]) / 255.0,
        }
    };
    if !matches!(
        format,
        TextureFormat::Rgba32Float
            | TextureFormat::Rgba16Float
            | TextureFormat::Rgba8UnormSrgb
            | TextureFormat::Rgba8Unorm
    ) {
        return Err(SkyImageError::Format(format));
    }
    let texels = data[..count * pixel]
        .chunks_exact(pixel)
        .map(|bytes| {
            let rgb = Vec3::new(channel(bytes, 0), channel(bytes, 1), channel(bytes, 2));
            // NaN and infinite texels would spread through every filtered level.
            if rgb.is_finite() {
                rgb.max(Vec3::ZERO)
            } else {
                Vec3::ZERO
            }
        })
        .collect();
    Ok(Equirect::new(width, height, texels))
}

/// An image for [`super::SkyEnvironment`] from radiance computed on the CPU, such as a
/// procedural sky. It stays on the CPU: only the sky's lighting data reaches the GPU.
pub fn source_image(sky: &Equirect) -> Image {
    let data = sky
        .texels
        .iter()
        .flat_map(|texel| [texel.x, texel.y, texel.z, 1.0])
        .flat_map(f32::to_le_bytes)
        .collect();
    let size = Extent3d {
        width: sky.width,
        height: sky.height,
        depth_or_array_layers: 1,
    };
    Image::new(
        size,
        TextureDimension::D2,
        data,
        TextureFormat::Rgba32Float,
        RenderAssetUsages::MAIN_WORLD,
    )
}

fn push_texels(bytes: &mut Vec<u8>, texels: &[Vec3]) {
    for texel in texels {
        let clamped = texel.min(Vec3::splat(F16_MAX));
        for value in [clamped.x, clamped.y, clamped.z, 1.0] {
            bytes.extend_from_slice(&f16::from_f32(value).to_le_bytes());
        }
    }
}

fn float_image(size: Extent3d, mips: u32, data: Vec<u8>) -> Image {
    let mut image = Image::new_uninit(
        size,
        TextureDimension::D2,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = mips;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor::linear());
    image
}

/// The specular cube as a cube image with one mip level per roughness level, faces then levels
/// in memory (layer-major).
pub fn specular_image(cube: &SpecularCube) -> Image {
    let mut data = Vec::new();
    for face in 0..CUBE_FACE_COUNT as usize {
        for (level, texels) in cube.levels.iter().enumerate() {
            let face_texels = (cube.level_resolution(level) as usize).pow(2);
            push_texels(
                &mut data,
                &texels[face * face_texels..(face + 1) * face_texels],
            );
        }
    }
    let size = Extent3d {
        width: cube.resolution,
        height: cube.resolution,
        depth_or_array_layers: CUBE_FACE_COUNT,
    };
    let mut image = float_image(size, cube.levels.len() as u32, data);
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}

/// The sky pyramid as an equirectangular image with its levels as mips, for the background.
pub fn background_image(pyramid: &Pyramid) -> Image {
    let mut data = Vec::new();
    for level in &pyramid.levels {
        push_texels(&mut data, &level.texels);
    }
    let base = pyramid.base();
    let size = Extent3d {
        width: base.width,
        height: base.height,
        depth_or_array_layers: 1,
    };
    float_image(size, pyramid.levels.len() as u32, data)
}

/// A black 1×1 cube, bound until a sky is ready.
pub fn placeholder_specular() -> Image {
    specular_image(&SpecularCube {
        resolution: 1,
        levels: vec![vec![Vec3::ZERO; CUBE_FACE_COUNT as usize]],
    })
}

/// A black 1×1 image, bound until a sky is ready.
pub fn placeholder_background() -> Image {
    background_image(&Pyramid::new(Equirect::new(1, 1, vec![Vec3::ZERO])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_and_srgb_images_read_as_linear() {
        let pixels: Vec<u8> = [2.0f32, 0.5, 0.25, 1.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let size = Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
        let image = Image::new(
            size,
            TextureDimension::D2,
            pixels,
            TextureFormat::Rgba32Float,
            RenderAssetUsages::default(),
        );
        let sky = equirect_from_image(&image).unwrap();
        assert_eq!(sky.texels, vec![Vec3::new(2.0, 0.5, 0.25)]);
        let srgb = Image::new(
            size,
            TextureDimension::D2,
            vec![255, 0, 188, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        let texel = equirect_from_image(&srgb).unwrap().texels[0];
        assert!((texel.x - 1.0).abs() < 1e-6 && texel.y == 0.0 && (texel.z - 0.5).abs() < 0.01);
        let gpu_only = Image {
            data: None,
            ..image
        };
        assert_eq!(
            equirect_from_image(&gpu_only),
            Err(SkyImageError::NoCpuData)
        );
    }

    #[test]
    fn the_cube_holds_every_face_and_level() {
        let cube = SpecularCube {
            resolution: 4,
            levels: vec![
                vec![Vec3::ONE; 6 * 16],
                vec![Vec3::ONE; 6 * 4],
                vec![Vec3::ONE; 6],
            ],
        };
        let image = specular_image(&cube);
        assert_eq!(image.texture_descriptor.mip_level_count, 3);
        assert_eq!(image.texture_descriptor.size.depth_or_array_layers, 6);
        assert_eq!(
            image.data.as_ref().map(Vec::len),
            Some(6 * (16 + 4 + 1) * 8)
        );
        let view = image
            .texture_view_descriptor
            .as_ref()
            .and_then(|v| v.dimension);
        assert_eq!(view, Some(TextureViewDimension::Cube));
        let placeholder = placeholder_specular();
        assert_eq!(placeholder.data.as_ref().map(Vec::len), Some(6 * 8));
    }

    #[test]
    fn the_background_keeps_its_levels_as_mips() {
        let pyramid = Pyramid::new(Equirect::from_fn(8, 4, |_| Vec3::ONE));
        let image = background_image(&pyramid);
        assert_eq!(image.texture_descriptor.mip_level_count, 4);
        let texels = 32 + 8 + 2 + 1;
        assert_eq!(image.data.as_ref().map(Vec::len), Some(texels * 8));
    }
}
