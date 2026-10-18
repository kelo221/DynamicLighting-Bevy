//! The outdoor sky: a procedural one by default, or an HDR image with `--hdri path` (EXR or
//! Radiance HDR), turned so its brightest point lines up with the sun's direction.

use core::f32::consts::PI;
use std::path::Path;

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, ImageSampler, ImageType};
use bevy::prelude::*;
use dynlight::environment::equirect::{Equirect, uv_to_direction};
use dynlight::environment::images::{equirect_from_image, source_image};
use dynlight::prelude::*;

/// Radiance above this is left out of the sky's light: the sun light stands in for the sun.
const MAX_SKY_RADIANCE: f32 = 3.0;
/// The mean radiance an HDR image is scaled to, about the procedural sky's.
const HDRI_MEAN_RADIANCE: f32 = 0.35;
/// The procedural sky's size; its light needs no more.
const PROCEDURAL_WIDTH: u32 = 1024;

/// Linear radiance of the procedural sky towards unit direction `d`, with the sun towards
/// `to_sun`: blue overhead, paler at the horizon, a glow and a disk around the sun, and a dim
/// ground.
fn procedural(d: Vec3, to_sun: Vec3, sun_radius: f32) -> Vec3 {
    let zenith = Vec3::new(0.10, 0.20, 0.45);
    let horizon = Vec3::new(0.45, 0.55, 0.65);
    let ground = Vec3::new(0.22, 0.2, 0.16);
    let along = d.dot(to_sun).max(0.0);
    let glow = Vec3::new(1.0, 0.85, 0.6) * (0.6 * along.powi(24) + 0.15 * along.powi(4));
    if d.y < 0.0 {
        let below = (-d.y * 8.0).min(1.0);
        return horizon.lerp(ground, below) + glow * (1.0 - below);
    }
    let disk = if along >= sun_radius.cos() {
        Vec3::new(400.0, 380.0, 340.0)
    } else {
        Vec3::ZERO
    };
    horizon.lerp(zenith, d.y.sqrt()) + glow + disk
}

/// The sky's image, light and turn.
struct Sky {
    image: Image,
    intensity: f32,
    max_radiance: f32,
    yaw: f32,
}

fn procedural_sky(to_sun: Vec3, sun_radius: f32) -> Sky {
    let height = PROCEDURAL_WIDTH / 2;
    let sky = Equirect::from_fn(PROCEDURAL_WIDTH, height, |d| {
        procedural(d, to_sun, sun_radius)
    });
    Sky {
        image: source_image(&sky),
        intensity: 1.0,
        max_radiance: MAX_SKY_RADIANCE,
        yaw: 0.0,
    }
}

/// Turning about +Y adds to this angle.
fn azimuth(d: Vec3) -> f32 {
    d.x.atan2(d.z)
}

fn hdri_sky(path: &str, to_sun: Vec3) -> Result<Sky, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    let extension = Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("exr");
    let image = Image::from_buffer(
        &bytes,
        ImageType::Extension(extension),
        CompressedImageFormats::NONE,
        false,
        ImageSampler::Default,
        RenderAssetUsages::MAIN_WORLD,
    )
    .map_err(|error| format!("{path}: {error}"))?;
    let sky = equirect_from_image(&image).map_err(|error| format!("{path}: {error}"))?;
    let (brightest, peak) = sky
        .texels
        .iter()
        .enumerate()
        .map(|(index, texel)| (index, texel.max_element()))
        .fold(
            (0, 0.0),
            |best, next| if next.1 > best.1 { next } else { best },
        );
    let uv = Vec2::new(
        ((brightest as u32 % sky.width) as f32 + 0.5) / sky.width as f32,
        ((brightest as u32 / sky.width) as f32 + 0.5) / sky.height as f32,
    );
    let toward = uv_to_direction(uv);
    // Scale the image so its sky, without the sun, is about as bright as the procedural one.
    let first_guess = HDRI_MEAN_RADIANCE / mean_radiance(&sky, f32::MAX).max(1.0e-6);
    let clamp = MAX_SKY_RADIANCE / first_guess;
    let intensity = HDRI_MEAN_RADIANCE / mean_radiance(&sky, clamp).max(1.0e-6);
    info!(
        "sky {path}: {}x{}, peak {peak:.0} towards {toward:.2}, intensity {intensity:.3}",
        sky.width, sky.height
    );
    Ok(Sky {
        image,
        intensity,
        max_radiance: MAX_SKY_RADIANCE / intensity,
        yaw: azimuth(to_sun) - azimuth(toward),
    })
}

/// The image's mean radiance over the sphere with every channel clamped at `max`.
fn mean_radiance(sky: &Equirect, max: f32) -> f32 {
    let mut sum = 0.0;
    for y in 0..sky.height {
        let solid_angle = sky.texel_solid_angle(y);
        for x in 0..sky.width {
            sum += sky.texel(x, y).min(Vec3::splat(max)).element_sum() / 3.0 * solid_angle;
        }
    }
    sum / (4.0 * PI)
}

/// Spawns the sky light: the image from `--hdri`, or the procedural sky.
pub fn spawn(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    sun: &DynamicLight,
    to_sun: Vec3,
) {
    let sun_radius = (sun.angular_diameter * 0.5).to_radians();
    let sky = crate::support::argument("--hdri")
        .and_then(|path| {
            hdri_sky(&path, to_sun)
                .inspect_err(|error| error!("--hdri: {error}; using the procedural sky"))
                .ok()
        })
        .unwrap_or_else(|| procedural_sky(to_sun, sun_radius));
    commands.spawn((
        Name::new("Sky"),
        SkyEnvironment {
            image: images.add(sky.image),
            max_radiance: Some(sky.max_radiance),
        },
        DynamicLight {
            intensity: sky.intensity,
            ..DynamicLight::sky()
        },
        Transform::from_rotation(Quat::from_rotation_y(sky.yaw)),
    ));
}
