//! GPU readback check: renders a floor lit by realtime point, spot, wave and interference
//! lights into a float image, reads it back and compares sampled pixels with the CPU mirror of
//! the light shaders (`dynlight::light::mirror`) evaluated on the same packed light records.
//!
//! `cargo run --example gpu_check` prints the largest difference and exits with an error code
//! when any sample is off by more than 1% (plus 0.01 absolute). The window shows the same view.

use bevy::camera::{Hdr, ImageRenderTarget, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use dynlight::light::PackedLights;
use dynlight::light::mirror::radiance;
use dynlight::prelude::*;

/// Image side in pixels: 256 `Rgba16Float` texels are 2048 bytes, so rows have no padding.
const SIZE: u32 = 256;
/// Side of the floor and of the orthographic view, in meters.
const EXTENT: f32 = 10.0;
/// Every `STEP`th pixel in each direction is checked.
const STEP: u32 = 8;
const RELATIVE_TOLERANCE: f32 = 0.01;
const ABSOLUTE_TOLERANCE: f32 = 0.01;
/// Real time before the readback is checked, so pipelines have compiled.
const WARM_UP_SECONDS: f32 = 2.0;

fn main() -> AppExit {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "DynLight GPU check".into(),
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

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(EXTENT, EXTENT))),
        MeshMaterial3d(materials.add(Color::WHITE)),
    ));
    let realtime = |light: DynamicLight| DynamicLight {
        realtime: true,
        ..light
    };
    commands.spawn((
        realtime(DynamicLight {
            color: Color::srgb(1.0, 0.6, 0.3),
            radius: 5.0,
            intensity: 1.5,
            falloff: 0.5,
            ..DynamicLight::point()
        }),
        Transform::from_xyz(-2.0, 1.5, -1.5),
    ));
    commands.spawn((
        realtime(DynamicLight {
            color: Color::srgb(0.3, 0.6, 1.0),
            radius: 7.0,
            inner_cutoff: 20.0,
            outer_cutoff: 32.0,
            ..DynamicLight::spot()
        }),
        Transform::from_xyz(2.0, 3.0, 1.0).looking_at(Vec3::new(2.5, 0.0, 1.5), Vec3::Z),
    ));
    commands.spawn((
        realtime(DynamicLight {
            color: Color::srgb(0.4, 1.0, 0.4),
            radius: 4.0,
            light_type: LightType::Wave,
            wave_speed: 0.0,
            wave_frequency: 3.0,
            wave_offset: 0.4,
            ..default()
        }),
        Transform::from_xyz(2.5, 1.0, -2.5),
    ));
    commands.spawn((
        realtime(DynamicLight {
            color: Color::srgb(1.0, 0.3, 0.8),
            radius: 4.0,
            light_type: LightType::Interference,
            wave_speed: 0.0,
            wave_frequency: 4.0,
            ..default()
        }),
        Transform::from_xyz(-2.5, 1.2, 2.5),
    ));

    let view = (
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: EXTENT,
                height: EXTENT,
            },
            ..OrthographicProjection::default_3d()
        }),
        // Image right is +X and image down is +Z.
        Transform::from_xyz(0.0, 10.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    );
    commands.spawn((
        Camera3d::default(),
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        RenderTarget::Image(ImageRenderTarget::from(target.clone())),
        view.clone(),
    ));
    // The same view in the window, to look at.
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            ..default()
        },
        view,
    ));
    commands.spawn(Readback::texture(target)).observe(check);
}

/// The world position seen at the center of pixel `(x, y)`.
fn pixel_world(x: u32, y: u32) -> Vec3 {
    let at = |pixel: u32| ((pixel as f32 + 0.5) / SIZE as f32 - 0.5) * EXTENT;
    Vec3::new(at(x), 0.0, at(y))
}

/// The floor color the lighting shaders should produce at `world`: white, no ambient, every
/// light unshadowed with the half-Lambert term of unbaked surfaces.
fn expected(lights: &PackedLights, world: Vec3) -> Vec3 {
    lights.lights.iter().fold(Vec3::ZERO, |sum, light| {
        let direction = (Vec3::from(light.position) - world).normalize_or_zero();
        let half_lambert = 0.5 + 0.5 * direction.y;
        sum + Vec3::from(light.color) * radiance(light, world) * half_lambert * half_lambert
    })
}

fn check(
    readback: On<ReadbackComplete>,
    time: Res<Time<Real>>,
    lights: Res<PackedLights>,
    mut exit: MessageWriter<AppExit>,
    mut done: Local<bool>,
) {
    if *done || time.elapsed_secs() < WARM_UP_SECONDS {
        return;
    }
    *done = true;
    let texels: Vec<half::f16> = bytemuck::cast_slice::<u8, u16>(&readback.data)
        .iter()
        .map(|&bits| half::f16::from_bits(bits))
        .collect();
    let mut worst = (0.0f32, UVec2::ZERO, Vec3::ZERO, Vec3::ZERO);
    let mut failures = 0;
    let mut samples = 0;
    for y in (STEP / 2..SIZE).step_by(STEP as usize) {
        for x in (STEP / 2..SIZE).step_by(STEP as usize) {
            let index = ((y * SIZE + x) * 4) as usize;
            let gpu = Vec3::from_array(core::array::from_fn(|c| texels[index + c].to_f32()));
            let cpu = expected(&lights, pixel_world(x, y));
            let error = (gpu - cpu).abs().max_element();
            let allowed = ABSOLUTE_TOLERANCE + RELATIVE_TOLERANCE * cpu.max_element();
            samples += 1;
            if error > allowed {
                failures += 1;
            }
            if error > worst.0 {
                worst = (error, UVec2::new(x, y), gpu, cpu);
            }
        }
    }
    let (error, pixel, gpu, cpu) = worst;
    info!(
        "{samples} samples of {} lights; largest difference {error:.4} at pixel {pixel} \
         (GPU {gpu:.4}, CPU {cpu:.4})",
        lights.lights.len()
    );
    if failures == 0 {
        info!("GPU lighting matches the CPU mirror");
        exit.write(AppExit::Success);
    } else {
        error!("{failures} of {samples} samples differ from the CPU mirror");
        exit.write(AppExit::error());
    }
}
