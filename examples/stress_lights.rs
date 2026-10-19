//! Stress test: 1000 lights over an 80 m floor, with up to two pillars beside each light.
//!
//! Baked (native default): `cargo run --release --example stress_lights -- --bake` bakes the
//! scene into `assets/stress_lights.dynlight`, shadows included; later runs load it. Without a
//! bake the lights render as realtime lights without shadows, of which only the nearest
//! `realtime_light_budget` count.
//!
//! Realtime (`--realtime`, and the browser default): every light is realtime and drifts, without
//! shadows. This measures the brute-force realtime loop instead of the baked per-triangle light
//! lists. (Realtime shadow cubemaps cost six depth renders per light per frame, far too many for
//! hundreds of lights.)
//!
//! - `--lights N` changes the number of lights (default 1000, 256 in a browser; rebake after
//!   changing it).
//! - `--frames N` exits after measuring N frames and prints the mean frame time;
//!   `--screenshot path.png` also saves the last frame. The camera holds still while measuring.
//! - `--capture path.png` saves a still frame for the visual baselines.
//! - `--single-threaded` adds `SingleThreadedSchedulesPlugin`, which cuts the frame time.
//!
//! Drag to orbit and scroll to zoom. VSync is off so the frame time is the real cost; browsers
//! cap the frame rate at the display's refresh rate.

#[path = "support/baking.rs"]
mod baking;
#[path = "support/capture.rs"]
mod capture;
#[path = "support/orbit.rs"]
mod orbit;
#[path = "support/mod.rs"]
mod support;

use core::f32::consts::TAU;

use baking::PendingBake;
use orbit::OrbitCamera;
use support::{argument, flag};

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::PresentMode;
use dynlight::bake::{BakeFinished, BakePreset};
use dynlight::baked_lighting::ActiveBake;
use dynlight::prelude::*;

const BAKE_ASSET: &str = "stress_lights.dynlight";
const FLOOR_SIZE: f32 = 80.0;
const LIGHT_RADIUS: f32 = 4.0;
const PILLAR_HEIGHT: f32 = 3.0;
/// Pillars stand this far from their light, so baked shadows cross its bright pool.
const PILLAR_DISTANCE: (f32, f32) = (0.8, 1.3);
/// How far a realtime light drifts from its place: round a circle, and up and down.
const DRIFT: Vec2 = Vec2::new(0.35, 0.3);
/// Real time before measuring starts: pipelines compile and the bake loads first.
const WARM_UP_SECONDS: f32 = 3.0;
/// The browser default: enough to stress the realtime loop without a long shader warm-up.
const WEB_LIGHTS: u32 = 256;

#[derive(Resource)]
struct Options {
    lights: u32,
    realtime: bool,
    frames: Option<u32>,
    screenshot: Option<String>,
}

#[derive(Resource, Default)]
struct Measurement {
    started_at: Option<f32>,
    frames: u32,
    seconds: f64,
}

/// A realtime light circling and bobbing around `anchor`.
#[derive(Component)]
struct Drift {
    anchor: Vec3,
    phase: f32,
    /// Radians per second; negative circles the other way.
    speed: f32,
}

fn main() -> AppExit {
    let web = cfg!(target_arch = "wasm32");
    let options = Options {
        lights: argument("--lights")
            .and_then(|value| value.parse().ok())
            .unwrap_or(if web { WEB_LIGHTS } else { 1000 }),
        realtime: flag("--realtime") || (web && !flag("--bake")),
        frames: argument("--frames").and_then(|value| value.parse().ok()),
        screenshot: argument("--screenshot"),
    };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            present_mode: PresentMode::AutoNoVsync,
            ..support::window("DynLight stress test")
        }),
        ..default()
    }))
    .add_plugins((
        DynamicLightingPlugin,
        support::scheduling(),
        FrameTimeDiagnosticsPlugin::default(),
    ))
    .add_plugins((baking::plugin, capture::plugin, orbit::plugin))
    .insert_resource(ClearColor(Color::srgb(0.01, 0.01, 0.015)))
    .insert_resource(BakeSettings {
        pixel_density: 16.0,
        ..default()
    })
    .insert_resource(DynamicLightingSettings {
        realtime_light_budget: if options.realtime { options.lights } else { 32 },
        ambient: Color::srgba(1.0, 1.0, 1.0, 0.05),
        ..default()
    })
    .init_resource::<Measurement>()
    .add_systems(Startup, (spawn_scene, load_or_bake))
    .add_systems(Update, (drift, apply_finished_bake, report))
    .insert_resource(options);
    app.run()
}

/// A small deterministic generator, so every run (and the bake) places the same scene.
fn hash(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    (x & 0x00ff_ffff) as f32 / 16_777_216.0
}

fn spawn_scene(
    mut commands: Commands,
    options: Res<Options>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    let mut floor = Plane3d::default()
        .mesh()
        .size(FLOOR_SIZE, FLOOR_SIZE)
        .subdivisions(39)
        .build();
    let uv = floor.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().clone();
    floor.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv);
    commands.spawn((
        Name::new("Floor"),
        Mesh3d(meshes.add(floor)),
        MeshMaterial3d(materials.add(Color::srgb(0.7, 0.7, 0.7))),
        BakeStatic,
    ));

    let mut pillar = Cylinder::new(0.3, PILLAR_HEIGHT).mesh().build();
    let uv = pillar.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().clone();
    pillar.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv);
    let pillar = meshes.add(pillar);
    let stone = materials.add(Color::srgb(0.6, 0.55, 0.5));

    // Lights sit on a jittered grid, each with up to two pillars beside it.
    let side = (options.lights as f32).sqrt().ceil() as u32;
    let spacing = FLOOR_SIZE / side as f32;
    let edge = FLOOR_SIZE / 2.0 - 0.5;
    for i in 0..options.lights {
        let cell = Vec2::new((i % side) as f32 + 0.5, (i / side) as f32 + 0.5) * spacing;
        let jitter = (Vec2::new(hash(i * 7 + 1), hash(i * 7 + 2)) - 0.5) * spacing * 0.4;
        let position = cell + jitter - FLOOR_SIZE / 2.0;
        let anchor = Vec3::new(position.x, 0.8 + hash(i + 104_729) * 1.5, position.y);
        let light = DynamicLight {
            color: Color::hsl(hash(i + 7919) * 360.0, 0.8, 0.6),
            radius: LIGHT_RADIUS,
            intensity: 1.5,
            realtime: options.realtime,
            ..DynamicLight::point()
        };
        let mut entity = commands.spawn((
            Name::new(format!("Light {i}")),
            light,
            Transform::from_translation(anchor),
        ));
        if i % 4 == 0 {
            entity.insert(LightEffect {
                kind: LightEffectKind::Candle,
                pulse_offset: hash(i + 31) * 10.0,
                ..default()
            });
        }
        // Baked lights must stay where they were baked.
        if options.realtime {
            entity.insert(Drift {
                anchor,
                phase: hash(i * 7 + 6) * TAU,
                speed: (0.3 + hash(i * 7 + 7) * 0.5) * if i % 2 == 0 { 1.0 } else { -1.0 },
            });
        }
        let pillars = (hash(i * 7 + 3) * 3.0) as u32;
        for k in 0..pillars {
            let angle = (hash(i * 7 + 4) + k as f32 / 2.0) * TAU;
            let (near, far) = PILLAR_DISTANCE;
            let away = Vec2::from_angle(angle) * (near + hash(i * 7 + 5 + k) * (far - near));
            let at = (position + away).clamp(Vec2::splat(-edge), Vec2::splat(edge));
            commands.spawn((
                Name::new(format!("Pillar {i}.{k}")),
                Mesh3d(pillar.clone()),
                MeshMaterial3d(stone.clone()),
                Transform::from_xyz(at.x, PILLAR_HEIGHT / 2.0, at.y),
                BakeStatic,
            ));
        }
    }

    let mut camera = OrbitCamera::new(Vec3::ZERO, 0.0, 0.5, 30.0);
    camera.distance_range = (6.0, 90.0);
    camera.pitch_range = (0.1, 1.45);
    camera.drift = if options.frames.is_some() || capture::active() {
        0.0
    } else {
        0.04
    };
    commands.spawn((Camera3d::default(), camera));
}

fn load_or_bake(
    options: Res<Options>,
    server: Res<AssetServer>,
    mut active: ResMut<ActiveBake>,
    mut pending: ResMut<PendingBake>,
) {
    if options.realtime {
        support::status(&format!("{} moving realtime lights", options.lights));
        return;
    }
    if flag("--bake") {
        pending.queue(BAKE_ASSET, BakePreset::Preview);
        return;
    }
    baking::load_existing(BAKE_ASSET, &server, &mut active);
    if active.0.is_none() {
        warn!("no bake yet: run with --bake; until then only the nearest lights render");
    }
    support::status(&format!("{} baked lights", options.lights));
}

fn drift(time: Res<Time>, mut lights: Query<(&Drift, &mut Transform)>) {
    let seconds = time.elapsed_secs();
    for (drift, mut transform) in &mut lights {
        let angle = drift.phase + seconds * drift.speed;
        transform.translation = drift.anchor
            + Vec3::new(
                angle.cos() * DRIFT.x,
                (angle * 1.7).sin() * DRIFT.y,
                angle.sin() * DRIFT.x,
            );
    }
}

fn apply_finished_bake(
    mut finished: MessageReader<BakeFinished>,
    mut active: ResMut<ActiveBake>,
    mut measurement: ResMut<Measurement>,
) {
    for bake in finished.read() {
        match &bake.result {
            Ok(report) => {
                info!(
                    "baked {} meshes, {} texels, {} rays in {} ms ({} bytes)",
                    report.stats.meshes,
                    report.stats.texels,
                    report.stats.rays,
                    report.milliseconds,
                    report.bytes
                );
                active.0 = Some(report.handle.clone());
                // Measure the baked scene, not the frames before it.
                *measurement = Measurement::default();
            }
            Err(error) => error!("bake failed: {error}"),
        }
    }
}

fn report(
    mut commands: Commands,
    options: Res<Options>,
    time: Res<Time<Real>>,
    diagnostics: Res<DiagnosticsStore>,
    mut measurement: ResMut<Measurement>,
    mut exit: MessageWriter<AppExit>,
    mut last_log: Local<f32>,
) {
    let now = time.elapsed_secs();
    let started_at = *measurement.started_at.get_or_insert(now + WARM_UP_SECONDS);
    if now < started_at {
        return;
    }
    measurement.frames += 1;
    measurement.seconds += time.delta_secs_f64();
    if now - *last_log > 1.0 {
        *last_log = now;
        if let Some(milliseconds) = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
            .and_then(|frame_time| frame_time.smoothed())
        {
            info!("{} lights: {milliseconds:.2} ms per frame", options.lights);
            support::page_text(
                "frame-time",
                &format!("{milliseconds:.2} ms ({:.0} fps)", 1000.0 / milliseconds),
            );
        }
    }
    let Some(frames) = options.frames else {
        return;
    };
    if measurement.frames == frames {
        if let Some(path) = &options.screenshot {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path.clone()));
        }
    } else if measurement.frames == frames + 5 {
        let mean = measurement.seconds * 1000.0 / f64::from(measurement.frames);
        info!(
            "{} {} lights: mean {mean:.3} ms per frame over {} frames",
            options.lights,
            if options.realtime {
                "realtime"
            } else {
                "baked"
            },
            measurement.frames
        );
        exit.write(AppExit::Success);
    }
}
