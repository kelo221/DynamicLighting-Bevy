//! Turns a probe's six captured faces into its lighting: the faces are read back, resampled into
//! an equirectangular image, and prefiltered and projected by the environment slice like a sky.
//! The cube array every lit material binds is assembled here too.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureViewDescriptor, TextureViewDimension};
use half::f16;

use crate::environment::equirect::{Equirect, Pyramid};
use crate::environment::images::specular_image;
use crate::environment::prefilter::{
    SPECULAR_LEVELS, SPECULAR_RESOLUTION, SPECULAR_SAMPLES, SpecularCube, prefilter,
};
use crate::environment::sh::{SH_COUNT, irradiance_over_pi, project};
use crate::shared::cube::{CUBE_FACE_COUNT, face_orientation};

/// Bytes per `Rgba16Float` texel.
const TEXEL_BYTES: usize = 8;
/// The widest image the irradiance is projected from; three bands need no more.
const SH_SOURCE_WIDTH: u32 = 256;

/// Bytes per row of a read-back texture `width` texels wide: rows are padded to 256 bytes.
pub fn padded_row_bytes(width: u32) -> usize {
    (width as usize * TEXEL_BYTES).div_ceil(256) * 256
}

/// The RGB texels of a read-back `Rgba16Float` face, row by row from the top. `None` when
/// `bytes` is too short.
pub fn read_face(bytes: &[u8], resolution: u32) -> Option<Vec<Vec3>> {
    let row = padded_row_bytes(resolution);
    let size = resolution as usize;
    if bytes.len() < row * size {
        return None;
    }
    let channel = |at: usize| f16::from_le_bytes([bytes[at], bytes[at + 1]]).to_f32();
    let mut texels = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let at = y * row + x * TEXEL_BYTES;
            let rgb = Vec3::new(channel(at), channel(at + 2), channel(at + 4));
            texels.push(if rgb.is_finite() {
                rgb.max(Vec3::ZERO)
            } else {
                Vec3::ZERO
            });
        }
    }
    Some(texels)
}

/// Where `direction` falls in the image of face `face`, in texels from the top left corner, as
/// a camera turned by `shared::cube::face_orientation` with a 90° field of view renders it.
/// `None` when it lies outside that face's view.
pub fn face_point(face: u32, direction: Vec3, resolution: u32) -> Option<Vec2> {
    let (forward, up) = face_orientation(face);
    let right = forward.cross(up);
    let depth = direction.dot(forward);
    if depth <= 0.0 {
        return None;
    }
    let x = direction.dot(right) / depth;
    let y = direction.dot(up) / depth;
    if x.abs() > 1.0 + 1e-4 || y.abs() > 1.0 + 1e-4 {
        return None;
    }
    let size = resolution as f32;
    Some(Vec2::new((x + 1.0) * 0.5 * size, (1.0 - y) * 0.5 * size))
}

/// The face whose view holds `direction`: the one its largest component points to.
fn face_of(direction: Vec3) -> u32 {
    let abs = direction.abs();
    if abs.x >= abs.y && abs.x >= abs.z {
        u32::from(direction.x < 0.0)
    } else if abs.y >= abs.z {
        2 + u32::from(direction.y < 0.0)
    } else {
        4 + u32::from(direction.z < 0.0)
    }
}

/// The captured radiance in `direction`, bilinearly filtered within its face.
pub fn sample_faces(faces: &[Vec<Vec3>], resolution: u32, direction: Vec3) -> Vec3 {
    let face = face_of(direction);
    let Some(point) = face_point(face, direction, resolution) else {
        return Vec3::ZERO;
    };
    let max = (resolution - 1) as f32;
    let x = (point.x - 0.5).clamp(0.0, max);
    let y = (point.y - 0.5).clamp(0.0, max);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(resolution - 1), (y0 + 1).min(resolution - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let texels = &faces[face as usize];
    let at = |x: u32, y: u32| texels[(y * resolution + x) as usize];
    let top = at(x0, y0).lerp(at(x1, y0), fx);
    let bottom = at(x0, y1).lerp(at(x1, y1), fx);
    top.lerp(bottom, fy)
}

/// The six faces as an equirectangular image four faces wide.
pub fn faces_to_equirect(faces: &[Vec<Vec3>], resolution: u32) -> Equirect {
    Equirect::from_fn(4 * resolution, 2 * resolution, |direction| {
        sample_faces(faces, resolution, direction)
    })
}

/// A captured probe, ready for the cube array and the light buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeLighting {
    pub specular: SpecularCube,
    /// Irradiance over π, in world space.
    pub irradiance: [Vec3; SH_COUNT],
}

/// The probe's prefiltered cube and irradiance from its six faces.
pub fn probe_lighting(faces: &[Vec<Vec3>], resolution: u32) -> ProbeLighting {
    let pyramid = Pyramid::new(faces_to_equirect(faces, resolution));
    let sh_level = pyramid
        .levels
        .iter()
        .find(|level| level.width <= SH_SOURCE_WIDTH)
        .unwrap_or(pyramid.base());
    ProbeLighting {
        specular: prefilter(
            &pyramid,
            SPECULAR_RESOLUTION,
            SPECULAR_LEVELS,
            SPECULAR_SAMPLES,
        ),
        irradiance: irradiance_over_pi(project(sh_level)),
    }
}

/// One cube of a cube image's data: per face, per level, the level's bytes.
fn cube_levels(image: &Image) -> Option<(u32, Vec<Vec<&[u8]>>)> {
    let descriptor = &image.texture_descriptor;
    let data = image.data.as_deref()?;
    let (resolution, levels) = (descriptor.size.width, descriptor.mip_level_count);
    if descriptor.size.depth_or_array_layers != CUBE_FACE_COUNT || resolution == 0 {
        return None;
    }
    let mut at = 0;
    let mut faces = Vec::new();
    for _ in 0..CUBE_FACE_COUNT {
        let mut face = Vec::new();
        for level in 0..levels {
            let side = (resolution >> level).max(1) as usize;
            let bytes = side * side * TEXEL_BYTES;
            face.push(data.get(at..at + bytes)?);
            at += bytes;
        }
        faces.push(face);
    }
    Some((resolution, faces))
}

/// `cube`'s data at `resolution` and `levels`, each texel taken from the nearest texel of the
/// level of the same size, or the smallest level.
fn resampled(cube: &Image, resolution: u32, levels: u32) -> Option<Vec<u8>> {
    let (source, faces) = cube_levels(cube)?;
    let mut data = Vec::new();
    for face in faces {
        for level in 0..levels {
            let side = (resolution >> level).max(1);
            let from = (level as usize).min(face.len() - 1);
            let from_side = (source >> from).max(1);
            for y in 0..side {
                for x in 0..side {
                    let (sx, sy) = (x * from_side / side, y * from_side / side);
                    let at = (sy * from_side + sx) as usize * TEXEL_BYTES;
                    data.extend_from_slice(&face[from][at..at + TEXEL_BYTES]);
                }
            }
        }
    }
    Some(data)
}

/// The cube array lit materials reflect: the sky's cube `sky` as cube 0, then the probes' cubes
/// in order. Without probes it is the sky's cube as it is; with them every cube has the probes'
/// size, and the sky is resampled to it.
pub fn cube_array(sky: &Image, probes: &[&SpecularCube]) -> Image {
    let (resolution, levels, mut data) = if probes.is_empty() {
        let size = sky.texture_descriptor.size.width;
        let levels = sky.texture_descriptor.mip_level_count;
        (size, levels, sky.data.clone().unwrap_or_default())
    } else {
        let (size, levels) = (SPECULAR_RESOLUTION, SPECULAR_LEVELS);
        (
            size,
            levels,
            resampled(sky, size, levels).unwrap_or_default(),
        )
    };
    let sky_bytes = (0..levels)
        .map(|level| ((resolution >> level).max(1) as usize).pow(2) * TEXEL_BYTES)
        .sum::<usize>()
        * CUBE_FACE_COUNT as usize;
    // A sky image without data (it can't be read) leaves its cube black.
    data.resize(sky_bytes, 0);
    for cube in probes {
        data.extend(specular_image(cube).data.unwrap_or_default());
    }
    let cubes = 1 + probes.len() as u32;
    let mut image = specular_image(&SpecularCube {
        resolution: 1,
        levels: vec![vec![Vec3::ZERO; CUBE_FACE_COUNT as usize]],
    });
    image.texture_descriptor.size = Extent3d {
        width: resolution,
        height: resolution,
        depth_or_array_layers: CUBE_FACE_COUNT * cubes,
    };
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::CubeArray),
        ..default()
    });
    image
}
