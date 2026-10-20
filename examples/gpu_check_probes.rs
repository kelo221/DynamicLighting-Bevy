//! GPU readback check of reflection probes: a closed room with six unlit walls in distinct
//! colors and a probe boxed to it, captured off-centre. A chrome floor quad (metallic, roughness
//! 0) must show, at every sampled pixel, the wall the CPU mirror of box projection
//! (`reflection_probes::records::box_projected`) predicts, which checks the capture's face
//! orientation, the probe's cube layer and the parallax correction together. A white quad
//! without bake data must be lit by the probe's irradiance (`environment::mirror::sky_irradiance`
//! on the same record), and that irradiance must match the one computed from the walls.
//!
//! `cargo run --example gpu_check_probes` prints the largest differences and exits with an error
//! code when a check fails. `--webgpu-limits` caps the device at WebGPU's default limits.

use bevy::camera::{Hdr, ImageRenderTarget, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::RenderPlugin;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::settings::{WgpuLimits, WgpuSettings};
use dynlight::environment::equirect::Equirect;
use dynlight::environment::mirror::sky_irradiance;
use dynlight::environment::sh::{evaluate, irradiance_over_pi, project};
use dynlight::light::PackedLights;
use dynlight::prelude::*;
use dynlight::reflection_probes::ReflectionProbeStatus;
use dynlight::reflection_probes::records::{box_point, box_projected, probe_at};
use dynlight::shared::GpuReflectionProbe;

/// Image side in pixels: 256 `Rgba16Float` texels are 2048 bytes, so rows have no padding.
const SIZE: u32 = 256;
/// Side of the orthographic view, in meters.
const EXTENT: f32 = 8.0;
const ROOM_CENTER: Vec3 = Vec3::new(0.0, 2.0, 0.0);
const ROOM_HALF: Vec3 = Vec3::new(3.0, 2.0, 4.0);
const CAPTURE_OFFSET: Vec3 = Vec3::new(0.8, 0.3, -0.6);
/// The camera looks straight down from inside the room, under the ceiling, so reflected rays
/// leave the floor at a slant and reach the side walls as well as the ceiling.
const CAMERA_HEIGHT: f32 = 3.5;
/// Height of the two floor quads.
const QUAD_HEIGHT: f32 = 0.02;
/// The chrome quad's and the white quad's extents along Z; both span `QUAD_X` along X.
const MIRROR_Z: [f32; 2] = [-3.8, 0.8];
const WHITE_Z: [f32; 2] = [1.4, 3.8];
const QUAD_X: f32 = 2.8;
/// The colors of the walls at +X, −X, +Y (ceiling), −Y (floor), +Z and −Z.
const WALLS: [Vec3; 6] = [
    Vec3::new(0.9, 0.15, 0.1),
    Vec3::new(0.1, 0.8, 0.2),
    Vec3::new(0.15, 0.3, 0.95),
    Vec3::new(0.4, 0.4, 0.4),
    Vec3::new(0.95, 0.85, 0.1),
    Vec3::new(0.8, 0.1, 0.7),
];
/// A reflected sample counts only when rays this far off its direction (radians) see the same
/// wall, so the prefiltered cube's blur across wall edges doesn't fail it: the floor's minimum
/// roughness mixes in some of the second, blurrier level.
const EDGE_MARGIN: f32 = 0.15;
const STEP: u32 = 4;
const RELATIVE_TOLERANCE: f32 = 0.02;
const ABSOLUTE_TOLERANCE: f32 = 0.01;
/// The probe's irradiance against the walls': the capture's 16-bit texels and its resampling
/// to an equirect image round it a little.
const IRRADIANCE_TOLERANCE: f32 = 0.03;
/// Walls that reflected samples must show, so the capture's orientation is tested.
const MIN_WALLS: usize = 4;
/// Reflected samples whose uncorrected direction shows another wall, so box projection is tested.
const MIN_CORRECTED: usize = 50;
const DEADLINE_SECONDS: f32 = 60.0;

fn main() -> AppExit {
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
                        title: "DynLight reflection probe GPU check".into(),
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

    // Each wall faces into the room; seen from behind, Bevy culls it.
    for (face, color) in WALLS.into_iter().enumerate() {
        let axis = face / 2;
        let sign = if face % 2 == 0 { 1.0 } else { -1.0 };
        let outward = Vec3::AXES[axis] * sign;
        let normal = Dir3::new(-outward).expect("an axis");
        let plane = Plane3d::new(*normal, Vec2::splat(ROOM_HALF.max_element()));
        commands.spawn((
            Mesh3d(meshes.add(plane.mesh())),
            MeshMaterial3d(materials.add(DynLitMaterial {
                base_color: LinearRgba::rgb(color.x, color.y, color.z).into(),
                unlit: true,
                ..default()
            })),
            Transform::from_translation(ROOM_CENTER + outward * ROOM_HALF[axis]),
        ));
    }
    let mut quad = |z: [f32; 2]| {
        let mesh = Plane3d::default().mesh().size(2.0 * QUAD_X, z[1] - z[0]);
        let at = Vec3::new(0.0, QUAD_HEIGHT, 0.5 * (z[0] + z[1]));
        (Mesh3d(meshes.add(mesh)), Transform::from_translation(at))
    };
    commands.spawn((
        quad(MIRROR_Z),
        MeshMaterial3d(materials.add(DynLitMaterial {
            base_color: Color::WHITE,
            shading: ShadingModel::Metallic,
            metallic: 1.0,
            perceptual_roughness: 0.0,
            ..default()
        })),
    ));
    commands.spawn((quad(WHITE_Z), MeshMaterial3d(materials.add(Color::WHITE))));
    commands.spawn((
        ReflectionProbe {
            capture_offset: CAPTURE_OFFSET,
            ..default()
        },
        Transform::from_translation(ROOM_CENTER).with_scale(ROOM_HALF),
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

/// The world position seen at the center of pixel `(x, y)`, on the floor quads.
fn pixel_world(x: u32, y: u32) -> Vec3 {
    let at = |pixel: u32| ((pixel as f32 + 0.5) / SIZE as f32 - 0.5) * EXTENT;
    Vec3::new(at(x), QUAD_HEIGHT, at(y))
}

/// Whether `world` lies on a quad spanning `z`, a pixel and a half inside its edges.
fn on_quad(world: Vec3, z: [f32; 2]) -> bool {
    let margin = 1.5 * EXTENT / SIZE as f32;
    world.x.abs() < QUAD_X - margin && world.z > z[0] + margin && world.z < z[1] - margin
}

/// What the probe's capture sees from `from` along `direction`: the index of the wall it hits,
/// or `None` for the floor quads, which are black there (nothing lights them yet).
fn seen(from: Vec3, direction: Vec3) -> Option<usize> {
    let local = from - ROOM_CENTER;
    let (face, t) = (0..3)
        .filter(|&axis| direction[axis].abs() > 1e-8)
        .map(|axis| {
            let sign = direction[axis].signum();
            let t = (sign * ROOM_HALF[axis] - local[axis]) / direction[axis];
            (2 * axis + usize::from(sign < 0.0), t)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("a nonzero direction");
    let quads = (direction.y < 0.0).then(|| (QUAD_HEIGHT - from.y) / direction.y);
    if let Some(quad_t) = quads.filter(|&quad_t| quad_t < t) {
        let hit = from + direction * quad_t;
        let within = |z: [f32; 2]| hit.z >= z[0] && hit.z <= z[1];
        if hit.x.abs() <= QUAD_X && (within(MIRROR_Z) || within(WHITE_Z)) {
            return None;
        }
    }
    Some(face)
}

/// The radiance the capture sees along `direction`.
fn radiance(capture: Vec3, direction: Vec3) -> Vec3 {
    seen(capture, direction).map_or(Vec3::ZERO, |wall| WALLS[wall])
}

/// The wall a reflected sample shows, unless rays `EDGE_MARGIN` off it see another one.
fn clear_wall(capture: Vec3, toward: Vec3) -> Option<usize> {
    let toward = toward.normalize();
    let (a, b) = toward.any_orthonormal_pair();
    let wall = seen(capture, toward)?;
    let offsets = [a, -a, b, -b];
    offsets
        .iter()
        .all(|&o| seen(capture, (toward + o * EDGE_MARGIN).normalize()) == Some(wall))
        .then_some(wall)
}

/// The record's irradiance at an upward normal, and the one the walls give at the capture point.
fn irradiance(record: &GpuReflectionProbe) -> (Vec3, Vec3) {
    let capture = Vec3::from_array(record.capture);
    let image = Equirect::from_fn(512, 256, |direction| radiance(capture, direction));
    let walls = evaluate(&irradiance_over_pi(project(&image)), Vec3::Y);
    (sky_irradiance(&record.irradiance, Vec3::Y), walls)
}

/// Differences found in a readback.
#[derive(Default)]
struct Findings {
    samples: usize,
    failures: usize,
    walls: [usize; 6],
    /// Samples that would show another wall without box projection.
    corrected: usize,
    worst: (f32, UVec2, Vec3, Vec3),
}

impl Findings {
    fn compare(&mut self, pixel: UVec2, gpu: Vec3, cpu: Vec3) {
        let error = (gpu - cpu).abs().max_element();
        let allowed = ABSOLUTE_TOLERANCE + RELATIVE_TOLERANCE * cpu.max_element();
        self.samples += 1;
        self.failures += usize::from(error > allowed);
        if error > self.worst.0 {
            self.worst = (error, pixel, gpu, cpu);
        }
    }
}

fn check(
    readback: On<ReadbackComplete>,
    time: Res<Time<Real>>,
    status: Res<ReflectionProbeStatus>,
    lights: Res<PackedLights>,
    mut exit: MessageWriter<AppExit>,
    mut done: Local<bool>,
) {
    let elapsed = time.elapsed_secs();
    let captured = status.captured == 1 && status.pending == 0 && lights.probes.count == 1;
    if *done || (!captured && elapsed < DEADLINE_SECONDS) {
        return;
    }
    let texels: Vec<half::f16> = bytemuck::cast_slice::<u8, u16>(&readback.data)
        .iter()
        .map(|&bits| half::f16::from_bits(bits))
        .collect();
    let gpu_at = |x: u32, y: u32| {
        let index = ((y * SIZE + x) * 4) as usize;
        Vec3::from_array(core::array::from_fn(|c| texels[index + c].to_f32()))
    };
    let record = &lights.probes.probes[0];
    let capture = Vec3::from_array(record.capture);
    let camera = Vec3::Y * CAMERA_HEIGHT;
    let (mut mirror, mut white) = (Findings::default(), Findings::default());
    for y in (STEP / 2..SIZE).step_by(STEP as usize) {
        for x in (STEP / 2..SIZE).step_by(STEP as usize) {
            let world = pixel_world(x, y);
            let pixel = UVec2::new(x, y);
            if on_quad(world, MIRROR_Z) {
                let view = (camera - world).normalize();
                let direction = (-view).reflect(Vec3::Y);
                let Some(wall) = clear_wall(capture, box_projected(record, world, direction))
                else {
                    continue;
                };
                mirror.walls[wall] += 1;
                mirror.corrected += usize::from(seen(capture, direction) != Some(wall));
                mirror.compare(pixel, gpu_at(x, y), WALLS[wall]);
            } else if on_quad(world, WHITE_Z) {
                let cpu = sky_irradiance(&record.irradiance, Vec3::Y);
                white.compare(pixel, gpu_at(x, y), cpu);
            }
        }
    }
    let (probe, walls) = if captured {
        irradiance(record)
    } else {
        (Vec3::ZERO, Vec3::ONE)
    };
    let irradiance_error = ((probe - walls).abs() / walls.max(Vec3::splat(1e-3))).max_element();
    let walls_seen = mirror.walls.iter().filter(|&&n| n > 0).count();
    let inside = probe_at(&lights.probes, ROOM_CENTER).is_some()
        && box_point(record, capture).abs().max_element() < 1.0;
    let passed = captured
        && inside
        && mirror.failures == 0
        && white.failures == 0
        && white.samples > 0
        && walls_seen >= MIN_WALLS
        && mirror.corrected >= MIN_CORRECTED
        && irradiance_error <= IRRADIANCE_TOLERANCE;
    if !passed && elapsed < DEADLINE_SECONDS {
        return;
    }
    *done = true;
    for (name, findings) in [("chrome", &mirror), ("white", &white)] {
        let (error, pixel, gpu, cpu) = findings.worst;
        info!(
            "after {elapsed:.1} s: {name} quad: {} samples, {} failures; largest difference \
             {error:.4} at pixel {pixel} (GPU {gpu:.4}, CPU {cpu:.4})",
            findings.samples, findings.failures
        );
    }
    info!(
        "reflected samples per wall {:?}, {} changed by box projection; probe irradiance {probe:.4} vs the walls' {walls:.4} \
         ({:.1}% off); status {status:?}",
        mirror.walls,
        mirror.corrected,
        100.0 * irradiance_error
    );
    if passed {
        info!("GPU reflection probes match the CPU mirror");
        exit.write(AppExit::Success);
    } else {
        error!("the reflection probe check failed");
        exit.write(AppExit::error());
    }
}
