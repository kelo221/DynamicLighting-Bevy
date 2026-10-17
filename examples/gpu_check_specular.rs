//! GPU readback check of Metallic shading's specular light: a copper floor, smoother on the left
//! than on the right, under realtime rectangle, tube and point lights, seen from a low camera so
//! the view grazes the floor toward the edges. Rectangles reflect through the LTC table, tubes
//! and points from one point. Sampled pixels are compared with the CPU mirrors
//! (`surface::mirror`, `light::ltc`) on the same packed light records.
//!
//! `cargo run --example gpu_check_specular` prints the largest difference and exits with an
//! error code when any sample is off by more than 1% (plus 0.01 absolute).

use bevy::camera::{Hdr, ImageRenderTarget, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use dynlight::light::area::AreaShape;
use dynlight::light::{PackedLights, mirror};
use dynlight::prelude::*;
use dynlight::surface::mirror::{Viewer, specular};

/// Image side in pixels: 256 `Rgba16Float` texels are 2048 bytes, so rows have no padding.
const SIZE: u32 = 256;
/// Side of the floor and of the orthographic view, in meters.
const EXTENT: f32 = 10.0;
/// Height of the camera: low, so the view vector tilts up to about 70° at the corners.
const CAMERA_HEIGHT: f32 = 2.0;
/// Roughness of the left and right floor halves.
const ROUGHNESS: [f32; 2] = [0.3, 0.65];
const COPPER: Vec3 = Vec3::new(0.95, 0.64, 0.54);
const STEP: u32 = 4;
const RELATIVE_TOLERANCE: f32 = 0.01;
const ABSOLUTE_TOLERANCE: f32 = 0.01;
/// Real time before readbacks are checked, and until a matching one may arrive (pipelines can
/// take a while to compile in debug builds, and Direct3D 12's FXC about 12 s).
const WARM_UP_SECONDS: f32 = 2.0;
const DEADLINE_SECONDS: f32 = 60.0;
/// Samples that must take a rectangle's LTC integral, so the table is tested.
const MIN_INTEGRATED: usize = 200;

fn main() -> AppExit {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "DynLight specular GPU check".into(),
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

    let half = meshes.add(Plane3d::default().mesh().size(EXTENT / 2.0, EXTENT));
    for (side, roughness) in [-1.0, 1.0].into_iter().zip(ROUGHNESS) {
        commands.spawn((
            Mesh3d(half.clone()),
            MeshMaterial3d(materials.add(DynLitMaterial {
                base_color: LinearRgba::rgb(COPPER.x, COPPER.y, COPPER.z).into(),
                shading: ShadingModel::Metallic,
                metallic: 1.0,
                perceptual_roughness: roughness,
                ..default()
            })),
            Transform::from_xyz(side * EXTENT / 4.0, 0.0, 0.0),
        ));
    }
    let realtime = |light: DynamicLight| DynamicLight {
        realtime: true,
        ..light
    };
    // A large panel facing down across the seam, a tilted one, a tube and a point light.
    commands.spawn((
        realtime(DynamicLight {
            color: Color::srgb(1.0, 0.95, 0.85),
            radius: 5.0,
            ..DynamicLight::rectangle(2.4, 1.2)
        }),
        Transform::from_xyz(0.0, 1.2, -1.5).looking_to(Dir3::NEG_Y, Vec3::NEG_Z),
    ));
    commands.spawn((
        realtime(DynamicLight {
            color: Color::srgb(0.7, 0.85, 1.0),
            radius: 4.0,
            ..DynamicLight::rectangle(1.0, 1.6)
        }),
        Transform::from_xyz(-2.5, 0.8, 2.5).looking_to(Vec3::new(0.5, -1.0, -0.3), Vec3::Z),
    ));
    commands.spawn((
        realtime(DynamicLight {
            color: Color::srgb(0.6, 1.0, 0.7),
            radius: 3.5,
            ..DynamicLight::tube(2.0)
        }),
        Transform::from_xyz(2.5, 0.7, 2.5).with_rotation(Quat::from_rotation_y(0.6)),
    ));
    commands.spawn((
        realtime(DynamicLight {
            color: Color::srgb(1.0, 0.5, 0.3),
            radius: 3.0,
            ..DynamicLight::point()
        }),
        Transform::from_xyz(3.0, 0.9, -3.0),
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
        Transform::from_xyz(0.0, CAMERA_HEIGHT, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
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

/// The floor color the shaders should produce at `world`: with metallic 1 and no ambient it is
/// the specular light alone. Also whether a rectangle reaching it was integrated.
fn expected(lights: &PackedLights, world: Vec3) -> (Vec3, bool) {
    let at = Viewer {
        world,
        normal: Vec3::Y,
        view: (Vec3::Y * CAMERA_HEIGHT - world).normalize(),
        roughness: ROUGHNESS[usize::from(world.x > 0.0)],
        metallic: 1.0,
        f0: COPPER,
    };
    lights
        .lights
        .iter()
        .fold((Vec3::ZERO, false), |(sum, integrated), light| {
            let strength = mirror::radiance(light, world);
            if strength == 0.0 || mirror::diffuse(light, world, Vec3::Y, false) == 0.0 {
                return (sum, integrated);
            }
            let rectangle = AreaShape::of(light).is_some_and(|shape| shape.rectangle);
            let light_specular = specular(light, &at);
            let color = Vec3::from(light.color) * strength;
            (sum + color * light_specular, integrated || rectangle)
        })
}

/// Pixels whose neighbourhood stays on one floor half.
fn checked(x: u32) -> bool {
    pixel_world(x, 0).x.abs() > 1.5 * EXTENT / SIZE as f32
}

fn check(
    readback: On<ReadbackComplete>,
    time: Res<Time<Real>>,
    lights: Res<PackedLights>,
    mut exit: MessageWriter<AppExit>,
    mut done: Local<bool>,
) {
    let elapsed = time.elapsed_secs();
    if *done || elapsed < WARM_UP_SECONDS {
        return;
    }
    let texels: Vec<half::f16> = bytemuck::cast_slice::<u8, u16>(&readback.data)
        .iter()
        .map(|&bits| half::f16::from_bits(bits))
        .collect();
    let mut worst = (0.0f32, UVec2::ZERO, Vec3::ZERO, Vec3::ZERO);
    let (mut failures, mut samples, mut integrated) = (0, 0, 0);
    for y in (STEP / 2..SIZE).step_by(STEP as usize) {
        for x in (STEP / 2..SIZE).step_by(STEP as usize) {
            if !checked(x) {
                continue;
            }
            let index = ((y * SIZE + x) * 4) as usize;
            let gpu = Vec3::from_array(core::array::from_fn(|c| texels[index + c].to_f32()));
            let (cpu, rectangle) = expected(&lights, pixel_world(x, y));
            let error = (gpu - cpu).abs().max_element();
            let allowed = ABSOLUTE_TOLERANCE + RELATIVE_TOLERANCE * cpu.max_element();
            samples += 1;
            integrated += usize::from(rectangle && cpu.max_element() > 0.05);
            failures += usize::from(error > allowed);
            if error > worst.0 {
                worst = (error, UVec2::new(x, y), gpu, cpu);
            }
        }
    }
    let passed = failures == 0 && integrated >= MIN_INTEGRATED;
    if !passed && elapsed < DEADLINE_SECONDS {
        return;
    }
    *done = true;
    let (error, pixel, gpu, cpu) = worst;
    info!(
        "after {elapsed:.1} s: {samples} samples ({integrated} lit by rectangles) of {} lights; \
         largest difference {error:.4} at pixel {pixel} (GPU {gpu:.4}, CPU {cpu:.4})",
        lights.lights.len()
    );
    if passed {
        info!("GPU specular light matches the CPU mirror");
        exit.write(AppExit::Success);
    } else {
        error!("{failures} of {samples} samples differ from the CPU mirror");
        exit.write(AppExit::error());
    }
}
