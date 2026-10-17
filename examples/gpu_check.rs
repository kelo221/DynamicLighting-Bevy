//! GPU readback check: renders a floor lit by realtime point, spot, wave, interference,
//! rectangle and tube lights into a float image, reads it back and compares sampled pixels with
//! the CPU mirror of the light shaders (`dynlight::light::mirror`) evaluated on the same packed
//! light records.
//!
//! `cargo run --example gpu_check` prints the largest difference and exits with an error code
//! when any sample is off by more than 1% (plus 0.01 absolute). The window shows the same view.
//!
//! The realtime light grid (`dynlight::light_clusters`) is always on here. `--many` checks it
//! harder: 200 small lights on a wide floor, seen in perspective, while the window camera, which
//! the grid is built for, sees only part of the floor, so the readback also covers fragments
//! outside the grid.
//!
//! `--webgpu-limits` caps the device at WebGPU's default limits (8 storage buffers per shader
//! stage, among others) instead of whatever this GPU offers, as the most limited browsers do.

use bevy::camera::{CameraProjection, Hdr, ImageRenderTarget, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::settings::{WgpuLimits, WgpuSettings};
use dynlight::effects::EffectRng;
use dynlight::light::PackedLights;
use dynlight::light::mirror;
use dynlight::light_clusters::{Cell, Grid, light_sphere};
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
/// `--many`: lights, and the side of their floor in meters.
const MANY_LIGHTS: usize = 200;
const MANY_EXTENT: f32 = 100.0;

/// Maps a checked pixel to the floor point it shows.
#[derive(Resource, Clone, Copy)]
enum FloorView {
    /// Straight down, `EXTENT` meters across.
    Top,
    /// In perspective, through this matrix.
    Perspective { world_from_clip: Mat4, eye: Vec3 },
}

fn main() -> AppExit {
    let many = std::env::args().any(|arg| arg == "--many");
    let webgpu_limits = std::env::args().any(|arg| arg == "--webgpu-limits");
    let render = RenderPlugin {
        render_creation: WgpuSettings {
            constrained_limits: webgpu_limits.then(WgpuLimits::default),
            ..default()
        }
        .into(),
        ..default()
    };
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "DynLight GPU check".into(),
                        resolution: (512, 512).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(render),
        )
        .add_plugins(DynamicLightingPlugin)
        .insert_resource(DynamicLightingSettings {
            ambient: Color::NONE,
            realtime_light_budget: 256,
            realtime_cluster_min_lights: 0,
            ..default()
        })
        .add_systems(Startup, if many { setup_many } else { setup })
        .run()
}

/// The float image the checked camera renders into.
fn readback_target(images: &mut Assets<Image>) -> Handle<Image> {
    let mut target = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba16Float, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    images.add(target)
}

/// The checked camera: HDR, no tonemapping, rendering into `target`.
fn readback_camera(target: Handle<Image>) -> impl Bundle {
    (
        Camera3d::default(),
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        RenderTarget::Image(ImageRenderTarget::from(target)),
    )
}

/// `--many`: small lights of every shape spread over a wide floor, in perspective.
fn setup_many(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    let target = readback_target(&mut images);
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(MANY_EXTENT, MANY_EXTENT))),
        MeshMaterial3d(materials.add(Color::WHITE)),
    ));
    let mut rng = EffectRng::new(17, 3);
    let mut range = |low: f32, high: f32| low + (high - low) * rng.next_f32();
    for index in 0..MANY_LIGHTS {
        let shape = match index % 4 {
            0 => DynamicLight::point(),
            1 => DynamicLight::spot(),
            2 => DynamicLight::rectangle(0.6, 0.4),
            _ => DynamicLight::tube(0.8),
        };
        let position = Vec3::new(range(-14.0, 14.0), range(0.3, 1.2), range(-26.0, 4.0));
        let light = DynamicLight {
            realtime: true,
            color: Color::hsl(range(0.0, 360.0), 0.8, 0.6),
            radius: range(1.2, 3.0),
            ..shape
        };
        let aim = position + Vec3::new(range(-0.5, 0.5), -1.0, range(-0.5, 0.5));
        commands.spawn((
            light,
            Transform::from_translation(position).looking_at(aim, Vec3::Z),
        ));
    }
    let projection = PerspectiveProjection::default();
    let eye = Vec3::new(0.0, 12.0, 10.0);
    let checked = Transform::from_translation(eye).looking_at(Vec3::new(0.0, 0.0, -6.0), Vec3::Y);
    let world_from_clip = checked.to_matrix() * projection.get_clip_from_view().inverse();
    commands.insert_resource(FloorView::Perspective {
        world_from_clip,
        eye,
    });
    commands.spawn((
        readback_camera(target.clone()),
        Projection::Perspective(projection.clone()),
        checked,
    ));
    // The grid is built for this camera, which sees only the near part of the floor.
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            ..default()
        },
        Projection::Perspective(projection),
        Transform::from_xyz(-4.0, 5.0, 8.0).looking_at(Vec3::new(-6.0, 0.0, -2.0), Vec3::Y),
    ));
    commands.spawn(Readback::texture(target)).observe(check);
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    let target = readback_target(&mut images);
    commands.insert_resource(FloorView::Top);

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
    // A tilted panel and a tube, so their mean cosines vary across the floor.
    commands.spawn((
        realtime(DynamicLight {
            color: Color::srgb(1.0, 1.0, 0.8),
            radius: 4.0,
            ..DynamicLight::rectangle(1.6, 0.8)
        }),
        Transform::from_xyz(0.0, 1.0, -3.5).looking_to(Vec3::new(0.3, -1.0, 0.4), Vec3::Z),
    ));
    commands.spawn((
        realtime(DynamicLight {
            color: Color::srgb(0.6, 0.9, 1.0),
            radius: 3.5,
            intensity: 1.2,
            ..DynamicLight::tube(2.4)
        }),
        Transform::from_xyz(0.5, 0.6, 3.5).with_rotation(Quat::from_rotation_y(0.4)),
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
    commands.spawn((readback_camera(target.clone()), view.clone()));
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

/// The floor point seen at the center of pixel `(x, y)`.
fn pixel_world(view: FloorView, x: u32, y: u32) -> Vec3 {
    let unit = |pixel: u32| (pixel as f32 + 0.5) / SIZE as f32;
    match view {
        FloorView::Top => Vec3::new(unit(x) - 0.5, 0.0, unit(y) - 0.5) * EXTENT,
        FloorView::Perspective {
            world_from_clip,
            eye,
        } => {
            let ndc = Vec3::new(unit(x) * 2.0 - 1.0, 1.0 - unit(y) * 2.0, 1.0);
            let direction = world_from_clip.project_point3(ndc) - eye;
            eye + direction * (-eye.y / direction.y)
        }
    }
}

/// The floor color the lighting shaders should produce at `world`: white, no ambient, every
/// light unshadowed with the half-Lambert term of unbaked surfaces.
fn expected(lights: &PackedLights, world: Vec3) -> Vec3 {
    lights.lights.iter().fold(Vec3::ZERO, |sum, light| {
        let diffuse = mirror::diffuse(light, world, Vec3::Y, false);
        sum + Vec3::from(light.color) * mirror::radiance(light, world) * diffuse
    })
}

fn check(
    readback: On<ReadbackComplete>,
    time: Res<Time<Real>>,
    view: Res<FloorView>,
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
    let realtime = &lights.lights[lights.globals.baked_count as usize..];
    let grid = lights
        .viewer
        .and_then(|viewer| Grid::new(viewer, realtime.iter().filter_map(light_sphere)));
    let mut worst = (0.0f32, UVec2::ZERO, Vec3::ZERO, Vec3::ZERO);
    let mut failures = 0;
    let mut samples = 0;
    let mut in_grid = 0;
    for y in (STEP / 2..SIZE).step_by(STEP as usize) {
        for x in (STEP / 2..SIZE).step_by(STEP as usize) {
            let index = ((y * SIZE + x) * 4) as usize;
            let gpu = Vec3::from_array(core::array::from_fn(|c| texels[index + c].to_f32()));
            let world = pixel_world(*view, x, y);
            if grid.is_some_and(|grid| grid.cell_at(world) != Cell::Outside) {
                in_grid += 1;
            }
            let cpu = expected(&lights, world);
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
        "{samples} samples of {} lights, {in_grid} inside the light grid; largest difference \
         {error:.4} at pixel {pixel} (GPU {gpu:.4}, CPU {cpu:.4})",
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
