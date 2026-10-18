//! The sky build in a headless app: it starts when the image loads, lands in the shared
//! resources, and runs again when the image changes.

use core::time::Duration;

use bevy::asset::{AssetPlugin, RenderAssetUsages};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::systems::{SkyBuilds, build_skies, insert_placeholders};
use super::{SKY_SPECULAR, SkyEnvironment, SkyStatus};
use crate::light::{DynamicLight, LightChoices, LightType};
use crate::shared::SkyIrradiance;

fn sky_image(radiance: f32) -> Image {
    let texel: Vec<u8> = [radiance, radiance, radiance, 1.0]
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    Image::new(
        Extent3d {
            width: 16,
            height: 8,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        texel.repeat(16 * 8),
        TextureFormat::Rgba32Float,
        RenderAssetUsages::default(),
    )
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_resource::<SkyIrradiance>()
        .init_resource::<SkyBuilds>()
        .init_resource::<SkyStatus>()
        .init_resource::<LightChoices>()
        .add_systems(Update, build_skies);
    insert_placeholders(&mut app);
    app
}

/// Makes `sky` the sky that lights the scene, as the light slice's choice would.
fn choose(app: &mut App, sky: Option<Entity>) {
    app.world_mut().resource_mut::<LightChoices>().sky = sky;
}

/// The constant term of the sky's irradiance once it is built, waiting up to a minute.
fn wait_for_irradiance(app: &mut App, previous: f32) -> f32 {
    for _ in 0..6000 {
        app.update();
        let level = app.world().resource::<SkyIrradiance>().0.sh[0][0];
        if level != previous {
            return level;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the sky was never built");
}

#[test]
fn the_sky_builds_and_rebuilds_when_its_image_changes() {
    let mut app = app();
    let image = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(sky_image(2.0));
    let sky = app
        .world_mut()
        .spawn(SkyEnvironment::new(image.clone()))
        .id();
    let light = app.world().get::<DynamicLight>(sky).map(|l| l.light_type);
    assert_eq!(
        light,
        Some(LightType::Sky),
        "the sky light comes with the image"
    );
    choose(&mut app, Some(sky));

    // A uniform sky of radiance 2 has irradiance over π 2, all in the constant term.
    let level = wait_for_irradiance(&mut app, 0.0);
    assert!((level * 0.282_095 - 2.0).abs() < 0.01, "{level}");
    let specular = app.world().resource::<Assets<Image>>().get(&SKY_SPECULAR);
    let mips = specular.map(|image| image.texture_descriptor.mip_level_count);
    assert_eq!(mips, Some(super::prefilter::SPECULAR_LEVELS));

    let _ = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .insert(&image, sky_image(4.0));
    let brighter = wait_for_irradiance(&mut app, level);
    assert!((brighter - 2.0 * level).abs() < 0.01 * level, "{brighter}");
}

#[test]
fn a_failed_build_drops_the_old_sky_and_says_so() {
    let mut app = app();
    let image = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(sky_image(2.0));
    let sky = app
        .world_mut()
        .spawn(SkyEnvironment::new(image.clone()))
        .id();
    choose(&mut app, Some(sky));
    let level = wait_for_irradiance(&mut app, 0.0);

    let mut volume = sky_image(4.0);
    volume.texture_descriptor.dimension = TextureDimension::D3;
    let _ = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .insert(&image, volume);
    assert_eq!(wait_for_irradiance(&mut app, level), 0.0, "no light");
    let status = *app.world().resource::<SkyStatus>();
    assert_eq!(
        status,
        SkyStatus {
            building: false,
            failed: true
        }
    );
    assert_eq!(first_specular_texel(&app), 0.0);

    let _ = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .insert(&image, sky_image(2.0));
    assert!((wait_for_irradiance(&mut app, 0.0) - level).abs() < 1e-4);
    assert!(!app.world().resource::<SkyStatus>().failed);
}

/// The constant term of the sky's irradiance, scaled to the radiance of a uniform sky.
fn uniform_level(app: &App) -> f32 {
    app.world().resource::<SkyIrradiance>().0.sh[0][0] * 0.282_095
}

/// The first channel of the specular cube's first texel.
fn first_specular_texel(app: &App) -> f32 {
    let images = app.world().resource::<Assets<Image>>();
    let data = images
        .get(&SKY_SPECULAR)
        .and_then(|image| image.data.as_deref());
    let bytes = data.expect("the specular cube has CPU data");
    half::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32()
}

#[test]
fn a_sky_without_an_image_is_uniform_white_until_an_image_sky_is_built() {
    let mut app = app();
    let plain = app.world_mut().spawn(DynamicLight::sky()).id();
    choose(&mut app, Some(plain));
    app.update();
    assert!((uniform_level(&app) - 1.0).abs() < 1e-5);
    assert_eq!(first_specular_texel(&app), 1.0);

    let image = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(sky_image(2.0));
    let sky = app.world_mut().spawn(SkyEnvironment::new(image)).id();
    choose(&mut app, Some(sky));
    app.update();
    assert_eq!(
        uniform_level(&app),
        0.0,
        "no light until the image is built"
    );
    assert_eq!(first_specular_texel(&app), 0.0);
    let level = wait_for_irradiance(&mut app, 0.0) * 0.282_095;
    assert!((level - 2.0).abs() < 0.01, "{level}");

    app.world_mut().despawn(sky);
    choose(&mut app, Some(plain));
    app.update();
    assert!((uniform_level(&app) - 1.0).abs() < 1e-5);
    assert_eq!(first_specular_texel(&app), 1.0);
}

#[test]
fn a_clamp_limits_the_light_but_not_the_background() {
    let image = sky_image(100.0);
    let data = super::systems::build_sky(&image, Some(10.0)).expect("a readable image");
    let (irradiance, background) = data.parts();
    assert!((irradiance.sh[0][0] * 0.282_095 - 10.0).abs() < 0.05);
    let first = half::f16::from_le_bytes([background[0], background[1]]).to_f32();
    assert_eq!(first, 100.0);
}
