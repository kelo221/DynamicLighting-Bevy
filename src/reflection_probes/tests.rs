//! Probe capture data: read-back faces, their resampling and the cube array.

use bevy::prelude::*;
use bevy::render::render_resource::TextureViewDimension;
use half::f16;

use super::assemble::{
    cube_array, face_point, faces_to_equirect, padded_row_bytes, read_face, sample_faces,
};
use crate::environment::equirect::Equirect;
use crate::environment::images::{specular_image, uniform_specular};
use crate::environment::prefilter::{SPECULAR_LEVELS, SPECULAR_RESOLUTION, SpecularCube};
use crate::environment::sh::{evaluate, irradiance_over_pi, project};
use crate::shared::cube::{CUBE_FACE_COUNT, face_orientation};

/// Smooth, different along every axis and sign.
fn radiance(d: Vec3) -> Vec3 {
    Vec3::new(
        1.0 + d.x,
        1.5 + d.y * d.y - 0.5 * d.z,
        1.0 - 0.7 * d.y + 0.2 * d.x * d.z,
    )
}

/// The faces a camera turned by `face_orientation` renders of `radiance`, row by row from the
/// top, sampled at pixel centers.
fn rendered_faces(resolution: u32) -> Vec<Vec<Vec3>> {
    (0..CUBE_FACE_COUNT)
        .map(|face| {
            let (forward, up) = face_orientation(face);
            let right = forward.cross(up);
            let size = resolution as f32;
            (0..resolution * resolution)
                .map(|index| {
                    let (px, py) = ((index % resolution) as f32, (index / resolution) as f32);
                    let x = (px + 0.5) / size * 2.0 - 1.0;
                    let y = 1.0 - (py + 0.5) / size * 2.0;
                    radiance((forward + right * x + up * y).normalize())
                })
                .collect()
        })
        .collect()
}

#[test]
fn read_back_rows_are_unpadded() {
    assert_eq!(padded_row_bytes(128), 1024);
    assert_eq!(padded_row_bytes(20), 256);
    let (resolution, row) = (20, padded_row_bytes(20));
    let mut bytes = vec![0u8; row * resolution as usize];
    let at = 3 * row + 5 * 8;
    for (i, value) in [0.5f32, 2.0, 8.0, 1.0].into_iter().enumerate() {
        bytes[at + 2 * i..at + 2 * i + 2].copy_from_slice(&f16::from_f32(value).to_le_bytes());
    }
    let texels = read_face(&bytes, resolution).unwrap();
    assert_eq!(texels[3 * 20 + 5], Vec3::new(0.5, 2.0, 8.0));
    assert_eq!(texels[0], Vec3::ZERO);
    assert!(
        read_face(&bytes[..row * 19], resolution).is_none(),
        "cut short"
    );
}

#[test]
fn every_direction_falls_in_the_face_that_rendered_it() {
    let resolution = 16;
    for face in 0..CUBE_FACE_COUNT {
        let (forward, up) = face_orientation(face);
        let right = forward.cross(up);
        // The top right of the image is up and right of the view.
        let corner = face_point(face, forward + (right + up) * 0.9, resolution).unwrap();
        assert!(corner.x > 15.0 && corner.y < 1.0, "face {face}: {corner}");
        assert!(face_point(face, -forward, resolution).is_none());
    }
}

#[test]
fn resampled_faces_keep_the_radiance_and_its_irradiance() {
    let resolution = 32;
    let faces = rendered_faces(resolution);
    for direction in [
        Vec3::X,
        Vec3::NEG_Y,
        Vec3::new(0.3, 0.5, -0.8),
        Vec3::new(-1.0, 1.0, 1.0),
    ] {
        let direction = direction.normalize();
        let sampled = sample_faces(&faces, resolution, direction);
        assert!(
            sampled.abs_diff_eq(radiance(direction), 0.02),
            "{direction}: {sampled}"
        );
    }
    let resampled = irradiance_over_pi(project(&faces_to_equirect(&faces, resolution)));
    let exact = irradiance_over_pi(project(&Equirect::from_fn(256, 128, radiance)));
    for n in [Vec3::Y, Vec3::NEG_X, Vec3::new(0.6, -0.48, 0.64)] {
        let (a, b) = (evaluate(&resampled, n), evaluate(&exact, n));
        assert!(
            a.abs_diff_eq(b, 0.01 * b.max_element()),
            "{n}: {a} against {b}"
        );
    }
}

fn texel(image: &Image, index: usize) -> Vec3 {
    let data = image.data.as_ref().unwrap();
    let channel = |c: usize| {
        let at = index * 8 + c * 2;
        f16::from_le_bytes([data[at], data[at + 1]]).to_f32()
    };
    Vec3::new(channel(0), channel(1), channel(2))
}

#[test]
fn the_sky_comes_first_and_the_probes_follow() {
    let sky = uniform_specular();
    let alone = cube_array(&sky, &[]);
    assert_eq!(
        alone.data, sky.data,
        "without probes the sky's cube as it is"
    );
    assert_eq!(alone.texture_descriptor.size.depth_or_array_layers, 6);
    let view = alone
        .texture_view_descriptor
        .as_ref()
        .and_then(|v| v.dimension);
    assert_eq!(view, Some(TextureViewDimension::CubeArray));

    let levels = (0..SPECULAR_LEVELS)
        .map(|level| {
            let side = (SPECULAR_RESOLUTION >> level).max(1);
            vec![Vec3::new(0.25, 0.5, 2.0); (6 * side * side) as usize]
        })
        .collect();
    let probe = SpecularCube {
        resolution: SPECULAR_RESOLUTION,
        levels,
    };
    let array = cube_array(&sky, &[&probe, &probe]);
    let size = array.texture_descriptor.size;
    assert_eq!(
        (size.width, size.depth_or_array_layers),
        (SPECULAR_RESOLUTION, 18)
    );
    assert_eq!(array.texture_descriptor.mip_level_count, SPECULAR_LEVELS);
    let cube_texels: usize = (0..SPECULAR_LEVELS)
        .map(|level| ((SPECULAR_RESOLUTION >> level).max(1) as usize).pow(2) * 6)
        .sum();
    assert_eq!(array.data.as_ref().map(Vec::len), Some(3 * cube_texels * 8));
    // The 1×1 white sky fills its whole cube; the probes' cubes are their own.
    assert_eq!(texel(&array, cube_texels - 1), Vec3::ONE);
    assert_eq!(texel(&array, cube_texels), Vec3::new(0.25, 0.5, 2.0));
    let probe_image = specular_image(&probe);
    assert_eq!(
        &array.data.as_ref().unwrap()[cube_texels * 8..2 * cube_texels * 8],
        probe_image.data.as_deref().unwrap()
    );
}
