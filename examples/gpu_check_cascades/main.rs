//! GPU readback check of the sun's realtime cascades: the lit shader's `sun_shadow` against the
//! CPU mirror (`realtime_shadows::sun_mirror`). A low sun with realtime shadows lights two
//! floors seen from above, the left one baked, the right one not. Each frame the cascades are
//! replaced by synthetic ones (`cases.rs`), and both floors sample a synthetic moments array, so
//! every floor point's share of the sun is known. The check steps through 0, 1, 2 and 3
//! cascades, then 3 that hold moving meshes only, and requires every way through `sun_shadow`
//! on both floors: each cascade's core, its blend into the next, its blend when the next misses
//! the point, the last cascade's fade, and the points outside. The sun is low enough that the
//! terminator dims it, which baked floors skip when the cascades hold moving meshes only.
//!
//! `cargo run --example gpu_check_cascades -- [--quality low|medium|high]` prints the largest
//! difference and exits with an error code when a sample is off. A sample is the floor's color
//! over its color without cascades; it passes when it lies within 0.01 of the CPU values over a
//! quarter pixel around its center.

mod cases;

use std::collections::BTreeMap;

use bevy::camera::{Hdr, ImageRenderTarget, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use dynlight::bake::{BakeFinished, BakePreset, BakeRequest};
use dynlight::baked_lighting::ActiveBake;
use dynlight::light::{LightAttachments, LightSystems};
use dynlight::prelude::*;
use dynlight::shared::{RuntimeQuality, SunShadowBlock};

use cases::{CASES, Way};

/// Image side in pixels: 256 `Rgba16Float` texels are 2048 bytes, so rows have no padding.
const SIZE: u32 = 256;
/// Side of the view, in meters; each floor is half of it.
const EXTENT: f32 = 10.0;
const STEP: u32 = 2;
const TOLERANCE: f32 = 0.01;
/// The bake leaves the texels at the baked floor's corners partly shadowed, so points this close
/// to the outer edges, in meters, aren't checked.
const BORDER: f32 = 0.25;
/// The sun's rays; the floor's cosine towards the sun is about 0.2, inside the terminator.
const SUN_RAYS: Vec3 = Vec3::new(-1.0, -0.2, 0.0);
/// Real time after the bake is applied before the first case is checked.
const WARM_UP_SECONDS: f32 = 2.0;
/// Frames after a case starts before its readbacks count, and the time it gets to match.
const CASE_FRAMES: u32 = 10;
const CASE_SECONDS: f32 = 20.0;
/// Samples each floor needs on every way through `sun_shadow` the case requires.
const MIN_PER_WAY: usize = 3;

fn main() -> AppExit {
    let quality = match std::env::args()
        .skip_while(|a| a != "--quality")
        .nth(1)
        .as_deref()
    {
        Some("low") => RuntimeQuality::Low,
        Some("medium") => RuntimeQuality::Medium,
        _ => RuntimeQuality::High,
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "DynLight sun cascade GPU check".into(),
                resolution: (512, 512).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(DynamicLightingPlugin)
        .insert_resource(DynamicLightingSettings {
            ambient: Color::NONE,
            quality,
            ..default()
        })
        .insert_resource(BakeSettings {
            pixel_density: 8.0,
            ..default()
        })
        // A floor whose pipeline isn't ready yet reads as unlit.
        .insert_resource(ClearColor(Color::BLACK))
        .init_resource::<Progress>()
        .add_systems(Startup, setup)
        .add_systems(Update, (request_bake, apply_bake))
        .add_systems(
            PostUpdate,
            write_cascades
                .after(LightSystems::Attach)
                .before(LightSystems::Pack),
        )
        .run()
}

#[derive(Resource, Default)]
struct Progress {
    frame: u32,
    /// When the bake was applied, in real seconds.
    applied_at: Option<f32>,
    case: usize,
    case_frame: u32,
    case_since: f32,
    /// Each floor's color sum without cascades, from the first case.
    unshadowed: Option<[f32; 2]>,
    /// The first case's sums and the frame they were first seen, to wait until they settle.
    settling: Option<([f32; 2], u32)>,
    done: bool,
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
    let floor = materials.add(DynLitMaterial {
        sun_cascades: images.add(cases::moments_image()),
        ..DynLitMaterial::from(Color::WHITE)
    });

    let mut baked = Plane3d::default().mesh().size(EXTENT / 2.0, EXTENT).build();
    let uv0 = baked.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().clone();
    baked.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv0);
    commands.spawn((
        Name::new("Baked floor"),
        Mesh3d(meshes.add(baked)),
        MeshMaterial3d(floor.clone()),
        Transform::from_xyz(-EXTENT / 4.0, 0.0, 0.0),
        BakeStatic,
    ));
    commands.spawn((
        Name::new("Dynamic floor"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(EXTENT / 2.0, EXTENT))),
        MeshMaterial3d(floor),
        Transform::from_xyz(EXTENT / 4.0, 0.0, 0.0),
    ));
    let (sun, transform) = sun_light(Dir3::new(SUN_RAYS).unwrap());
    commands.spawn((
        Name::new("Sun"),
        DynamicLight {
            shadows: ShadowMode::Realtime,
            ..sun
        },
        transform,
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

/// Bakes once the scene's transforms are up to date.
fn request_bake(mut frames: Local<u32>, mut requests: MessageWriter<BakeRequest>) {
    *frames += 1;
    if *frames == 3 {
        requests.write(BakeRequest {
            output: None,
            preset: BakePreset::Preview,
        });
    }
}

fn apply_bake(
    mut finished: MessageReader<BakeFinished>,
    mut active: ResMut<ActiveBake>,
    mut progress: ResMut<Progress>,
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
) {
    for bake in finished.read() {
        match &bake.result {
            Ok(report) => {
                active.0 = Some(report.handle.clone());
                progress.applied_at = Some(time.elapsed_secs());
                progress.case_since = time.elapsed_secs() + WARM_UP_SECONDS;
            }
            Err(failure) => {
                error!("bake failed: {failure}");
                exit.write(AppExit::error());
            }
        }
    }
}

/// Replaces the cascades the realtime-shadows slice fitted with the current case's.
fn write_cascades(mut progress: ResMut<Progress>, mut block: ResMut<SunShadowBlock>) {
    progress.frame += 1;
    block.0 = cases::block(CASES[progress.case.min(CASES.len() - 1)]);
}

/// The world position seen at pixel coordinates `(x, y)` (pixel centers at `.5`).
fn pixel_world(x: f32, y: f32) -> Vec3 {
    let at = |pixel: f32| (pixel / SIZE as f32 - 0.5) * EXTENT;
    Vec3::new(at(x), 0.0, at(y))
}

/// What one readback showed for the current case.
#[derive(Default)]
struct Outcome {
    samples: usize,
    failures: usize,
    worst: (f32, Vec2),
    /// Samples per floor (0 baked, 1 dynamic) and way through `sun_shadow`.
    ways: BTreeMap<(usize, Way), usize>,
    /// Each floor's median color sum.
    sums: [f32; 2],
}

fn evaluate(texels: &[half::f16], case: cases::Case, unshadowed: [f32; 2]) -> Outcome {
    let cosine = -SUN_RAYS.normalize().y;
    let margin = 1.5 * EXTENT / SIZE as f32;
    let mut outcome = Outcome::default();
    let mut sums: [Vec<f32>; 2] = Default::default();
    for y in (STEP / 2..SIZE).step_by(STEP as usize) {
        for x in (STEP / 2..SIZE).step_by(STEP as usize) {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let world = pixel_world(px, py);
            let edge = EXTENT / 2.0 - BORDER;
            if world.x.abs() <= margin
                || world.x.abs().max(world.z.abs()) >= edge
                || !cases::clear_of_splits(world, margin)
            {
                continue;
            }
            let floor = usize::from(world.x > 0.0);
            let index = ((y * SIZE + x) * 4) as usize;
            let sum: f32 = (0..3).map(|c| texels[index + c].to_f32()).sum();
            sums[floor].push(sum);
            let around = [
                (0.0, 0.0),
                (-0.25, 0.0),
                (0.25, 0.0),
                (0.0, -0.25),
                (0.0, 0.25),
            ];
            let cpu = around.map(|(dx, dy)| {
                cases::expected(case, pixel_world(px + dx, py + dy), cosine, floor == 0)
            });
            let low = cpu.iter().copied().fold(f32::MAX, f32::min);
            let high = cpu.iter().copied().fold(f32::MIN, f32::max);
            let gpu = sum / unshadowed[floor].max(1.0e-6);
            let error = (low - gpu).max(gpu - high).max(0.0);
            outcome.samples += 1;
            outcome.failures += usize::from(error > TOLERANCE);
            if error > outcome.worst.0 {
                outcome.worst = (error, Vec2::new(px, py));
            }
            *outcome
                .ways
                .entry((floor, cases::way(case, world)))
                .or_default() += 1;
        }
    }
    for (floor, mut values) in sums.into_iter().enumerate() {
        values.sort_by(f32::total_cmp);
        outcome.sums[floor] = values.get(values.len() / 2).copied().unwrap_or(0.0);
    }
    outcome
}

fn check(
    readback: On<ReadbackComplete>,
    time: Res<Time<Real>>,
    attachments: Res<LightAttachments>,
    mut progress: ResMut<Progress>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    if progress.done
        || progress.applied_at.is_none()
        || now < progress.case_since
        || progress.frame < progress.case_frame + CASE_FRAMES
    {
        return;
    }
    let case = CASES[progress.case];
    let texels: Vec<half::f16> = bytemuck::cast_slice::<u8, u16>(&readback.data)
        .iter()
        .map(|&bits| half::f16::from_bits(bits))
        .collect();
    // The first case has no cascades, so it measures the floors' color without them, once the
    // pipelines are ready and the color has stayed the same for a while.
    let unshadowed = match progress.unshadowed {
        Some(sums) => sums,
        None => evaluate(&texels, case, [1.0; 2]).sums,
    };
    let frame = progress.frame;
    let settled = progress.unshadowed.is_some()
        || match progress.settling {
            Some((sums, since)) if sums == unshadowed => frame >= since + CASE_FRAMES,
            _ => {
                progress.settling = Some((unshadowed, frame));
                false
            }
        };
    let outcome = evaluate(&texels, case, unshadowed);
    let missing: Vec<String> = (0..2)
        .flat_map(|floor| {
            cases::required(case)
                .into_iter()
                .map(move |way| (floor, way))
        })
        .filter(|key| outcome.ways.get(key).copied().unwrap_or(0) < MIN_PER_WAY)
        .map(|(floor, way)| format!("{} floor {way:?}", ["baked", "dynamic"][floor]))
        .collect();
    let has_cascades = attachments.sun_cascades.is_some();
    let lit = unshadowed.iter().all(|&sum| sum > 0.1);
    let passed = outcome.failures == 0 && missing.is_empty() && has_cascades && lit && settled;
    if !passed && now - progress.case_since < CASE_SECONDS {
        return;
    }
    let ways: usize = outcome.ways.len();
    info!(
        "case {}: {} cascades, moving meshes only {}: {} samples on {ways} floor ways, largest \
         difference {:.4} at pixel {}",
        progress.case,
        case.count,
        case.moving_only,
        outcome.samples,
        outcome.worst.0,
        outcome.worst.1
    );
    if !passed {
        error!(
            "{} of {} samples differ from the CPU mirror; sun has cascades: {has_cascades}; floors \
             lit: {lit}; too few samples on: {}",
            outcome.failures,
            outcome.samples,
            missing.join(", ")
        );
        progress.done = true;
        exit.write(AppExit::error());
        return;
    }
    progress.unshadowed = Some(unshadowed);
    progress.case += 1;
    progress.case_frame = progress.frame;
    progress.case_since = now;
    if progress.case == CASES.len() {
        progress.done = true;
        info!("GPU sun cascades match the CPU mirror");
        exit.write(AppExit::Success);
    }
}
