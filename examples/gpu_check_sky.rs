//! GPU readback check of the sky light: a turned, tinted sky over a white Diffuse ball and a
//! Metallic ball, seen through a perspective camera with the sky behind them. Sampled pixels are
//! compared with the CPU mirror (`environment::mirror`): the balls' spherical-harmonic diffuse
//! light and prefiltered reflection, and the background against the sky's own formula.
//!
//! `cargo run --example gpu_check_sky` prints the largest difference and exits with an error
//! code when a sample is off. A sample passes when it lies within its tolerance of the CPU
//! values over a quarter pixel around its center.

use core::f32::consts::FRAC_PI_4;

use bevy::camera::{Hdr, ImageRenderTarget, RenderTarget};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use dynlight::environment::equirect::{Equirect, Pyramid};
use dynlight::environment::images::source_image;
use dynlight::environment::mirror::{fresnel_schlick_roughness, sample_specular, sky_irradiance};
use dynlight::environment::prefilter::{
    SPECULAR_LEVELS, SPECULAR_RESOLUTION, SPECULAR_SAMPLES, SpecularCube, prefilter,
};
use dynlight::environment::{SkyFrame, SkyStatus, first_sky};
use dynlight::light::PackedLights;
use dynlight::prelude::*;
use dynlight::shared::cube::direction_to_face_uv;

/// Image side in pixels: 256 `Rgba16Float` texels are 2048 bytes, so rows have no padding.
const SIZE: u32 = 256;
const FOV: f32 = FRAC_PI_4;
const STEP: u32 = 4;
const WARM_UP_SECONDS: f32 = 1.0;
const DEADLINE_SECONDS: f32 = 60.0;
const BALL_RADIUS: f32 = 1.0;
const DIFFUSE_BALL: Vec3 = Vec3::new(-1.15, 0.0, -4.0);
const METAL_BALL: Vec3 = Vec3::new(1.15, 0.0, -4.0);
const METAL_COLOR: Vec3 = Vec3::new(0.9, 0.8, 0.6);
const METAL_ROUGHNESS: f32 = 0.3;
/// Relative and absolute tolerance of the diffuse light and the background; the reflection,
/// mirrored without the GPU's seamless cube filtering, gets three times as much.
const TOLERANCE: (f32, f32) = (0.01, 0.005);

/// A smooth sky, so bilinear filtering and mip levels barely change it.
fn radiance(d: Vec3) -> Vec3 {
    Vec3::new(
        0.6 + 0.4 * d.y,
        0.35 + 0.25 * d.x,
        0.25 + 0.2 * d.z + 0.15 * d.y * d.y,
    )
}

#[derive(Resource)]
struct Reference(SpecularCube);

fn main() -> AppExit {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "DynLight sky GPU check".into(),
                resolution: (512, 512).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(DynamicLightingPlugin)
        .insert_resource(DynamicLightingSettings {
            ambient: Color::NONE,
            ..default()
        })
        .add_systems(Startup, setup)
        .run()
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    let mut target = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba16Float, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = images.add(target);

    let sky = Equirect::from_fn(512, 256, radiance);
    let pyramid = Pyramid::new(sky.clone());
    commands.insert_resource(Reference(prefilter(
        &pyramid,
        SPECULAR_RESOLUTION,
        SPECULAR_LEVELS,
        SPECULAR_SAMPLES,
    )));
    commands.spawn((
        Name::new("Sky"),
        SkyEnvironment::new(images.add(source_image(&sky))),
        DynamicLight {
            color: Color::srgb(1.0, 0.9, 0.8),
            intensity: 1.5,
            ..DynamicLight::sky()
        },
        Transform::from_rotation(Quat::from_rotation_y(0.7) * Quat::from_rotation_x(0.3)),
    ));

    let ball = meshes.add(Sphere::new(BALL_RADIUS).mesh().uv(96, 64));
    commands.spawn((
        Mesh3d(ball.clone()),
        MeshMaterial3d(materials.add(Color::WHITE)),
        Transform::from_translation(DIFFUSE_BALL),
    ));
    let metal = DynLitMaterial {
        shading: ShadingModel::Metallic,
        metallic: 1.0,
        perceptual_roughness: METAL_ROUGHNESS,
        ..DynLitMaterial::from(Color::linear_rgb(
            METAL_COLOR.x,
            METAL_COLOR.y,
            METAL_COLOR.z,
        ))
    };
    commands.spawn((
        Mesh3d(ball),
        MeshMaterial3d(materials.add(metal)),
        Transform::from_translation(METAL_BALL),
    ));

    let projection = Projection::Perspective(PerspectiveProjection {
        fov: FOV,
        aspect_ratio: 1.0,
        ..default()
    });
    commands.spawn((
        Camera3d::default(),
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        RenderTarget::Image(ImageRenderTarget::from(target.clone())),
        projection.clone(),
    ));
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            ..default()
        },
        projection,
    ));
    commands.spawn(Readback::texture(target)).observe(check);
}

/// The camera ray through pixel coordinates `(x, y)` (pixel centers at `.5`).
fn ray(x: f32, y: f32) -> Vec3 {
    let scale = (FOV * 0.5).tan();
    let ndc = Vec2::new(x / SIZE as f32 * 2.0 - 1.0, 1.0 - y / SIZE as f32 * 2.0);
    Vec3::new(ndc.x * scale, ndc.y * scale, -1.0).normalize()
}

/// Where the ray from the origin along `direction` first meets the ball at `center`.
fn hit(direction: Vec3, center: Vec3) -> Option<Vec3> {
    let along = direction.dot(center);
    let off = (center - direction * along).length_squared();
    let inside = BALL_RADIUS * BALL_RADIUS - off;
    (inside > 0.0).then(|| direction * (along - inside.sqrt()))
}

/// What the camera sees along `direction`: which surface, and the color the shaders should
/// produce there. The reflection is `None` when it falls near a cube face edge, where the GPU
/// filters across faces and the mirror doesn't.
fn expected(
    direction: Vec3,
    frame: &SkyFrame,
    lights: &PackedLights,
    cube: &SpecularCube,
) -> (usize, Option<Vec3>) {
    if let Some(point) = hit(direction, DIFFUSE_BALL) {
        let normal = (point - DIFFUSE_BALL).normalize();
        let light = sky_irradiance(&lights.sky, frame.to_local(normal)) * frame.tint;
        return (0, Some(light));
    }
    if let Some(point) = hit(direction, METAL_BALL) {
        let normal = (point - METAL_BALL).normalize();
        let view = -direction;
        let reflection = (2.0 * normal.dot(view)) * normal - view;
        let local = frame.to_local(reflection);
        let uv = direction_to_face_uv(local).1;
        if (uv - 0.5).abs().max_element() > 0.4 {
            return (1, None);
        }
        let lod = METAL_ROUGHNESS * (SPECULAR_LEVELS - 1) as f32;
        let fresnel =
            fresnel_schlick_roughness(normal.dot(view).max(0.0), METAL_COLOR, METAL_ROUGHNESS);
        let reflected = sample_specular(cube, local, lod) * frame.tint * fresnel;
        return (1, Some(reflected));
    }
    (2, Some(radiance(frame.to_local(direction)) * frame.tint))
}

#[allow(clippy::too_many_arguments)]
fn check(
    readback: On<ReadbackComplete>,
    time: Res<Time<Real>>,
    status: Res<SkyStatus>,
    lights: Res<PackedLights>,
    reference: Res<Reference>,
    mut ready_at: Local<Option<f32>>,
    mut exit: MessageWriter<AppExit>,
    mut done: Local<bool>,
) {
    let now = time.elapsed_secs();
    let Some(frame) = first_sky(&lights.globals, &lights.lights) else {
        return;
    };
    if status.building || lights.sky.sh[0][0] == 0.0 {
        *ready_at = None;
        return;
    }
    let since = now - *ready_at.get_or_insert(now);
    if *done || since < WARM_UP_SECONDS {
        return;
    }
    let texels: Vec<half::f16> = bytemuck::cast_slice::<u8, u16>(&readback.data)
        .iter()
        .map(|&bits| half::f16::from_bits(bits))
        .collect();
    let (mut worst, mut failures) = ((0.0, Vec2::ZERO), 0);
    let mut samples = [0usize; 3];
    for y in (STEP / 2..SIZE).step_by(STEP as usize) {
        for x in (STEP / 2..SIZE).step_by(STEP as usize) {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let around = [
                (0.0, 0.0),
                (-0.25, 0.0),
                (0.25, 0.0),
                (0.0, -0.25),
                (0.0, 0.25),
            ];
            let cpu: Vec<_> = around
                .iter()
                .map(|(dx, dy)| expected(ray(px + dx, py + dy), &frame, &lights, &reference.0))
                .collect();
            let surface = cpu[0].0;
            let colors: Option<Vec<Vec3>> = cpu
                .iter()
                .map(|(other, color)| color.filter(|_| *other == surface))
                .collect();
            let Some(colors) = colors else {
                continue;
            };
            let index = ((y * SIZE + x) * 4) as usize;
            let gpu = Vec3::from_array(core::array::from_fn(|c| texels[index + c].to_f32()));
            let low = colors.iter().fold(Vec3::MAX, |a, b| a.min(*b));
            let high = colors.iter().fold(Vec3::MIN, |a, b| a.max(*b));
            let scale = if surface == 1 { 3.0 } else { 1.0 };
            let allowed = scale * (TOLERANCE.1 + TOLERANCE.0 * high.max_element());
            let error = (low - gpu).max(gpu - high).max(Vec3::ZERO).max_element();
            samples[surface] += 1;
            failures += usize::from(error > allowed);
            if error / allowed > worst.0 {
                worst = (error / allowed, Vec2::new(px, py));
            }
        }
    }
    let passed = failures == 0 && samples.iter().all(|&n| n >= 50);
    if !passed && since < DEADLINE_SECONDS {
        return;
    }
    *done = true;
    info!(
        "{} diffuse, {} reflection and {} background samples; largest difference {:.2} of its \
         tolerance at pixel {}",
        samples[0], samples[1], samples[2], worst.0, worst.1
    );
    if passed {
        info!("GPU sky light matches the CPU mirror");
        exit.write(AppExit::Success);
    } else {
        error!("{failures} samples differ from the CPU mirror");
        exit.write(AppExit::error());
    }
}
