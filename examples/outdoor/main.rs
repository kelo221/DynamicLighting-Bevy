//! Outdoors under a baked sun and a baked sky: a courtyard with buildings, a porch, a pergola and
//! trees whose soft sun shadows and sky shadows (how much of the sky each texel sees) are baked
//! per texel, a lamp post, metal balls reflecting the sky, and a rolling ball and a spinning crate
//! that take the static scene's sun shadows from the sun map and its sky shadows from the sky
//! probes. The sun has realtime shadows on top of its baked ones: cascades of only the ball and
//! the crate, so they cast shadows onto the courtyard. The sky lights everything from all
//! directions and is drawn behind the scene; turning it or swapping its image keeps the bake.
//!
//! - `cargo run --example outdoor -- --bake` bakes (Full, a few seconds) and writes
//!   `assets/outdoor.dynlight`; later runs load it. Without a bake the sky shines unshadowed
//!   and the sun has only its realtime shadows.
//! - `--hdri path.exr` lights the scene with an HDR image (EXR or Radiance HDR) instead of the
//!   procedural sky, turned so its brightest point faces the sun's direction.
//! - `--frames N` exits after N frames, and `--screenshot path.png` saves the last ones.
//! - `--camera x,y,z,target_x,target_y,target_z` holds the camera still there.
//! - `--capture path.png` saves a still frame for the visual baselines.
//!
//! Drag to orbit, scroll to zoom.

#[path = "../support/baking.rs"]
mod baking;
#[path = "../support/capture.rs"]
mod capture;
#[path = "../support/lightmap_uvs.rs"]
mod lightmap_uvs;
#[path = "../support/orbit.rs"]
mod orbit;
mod scene;
mod sky;
#[path = "../support/mod.rs"]
mod support;

use baking::PendingBake;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use dynlight::bake::{BakeFinished, BakePreset};
use dynlight::baked_lighting::ActiveBake;
use dynlight::prelude::*;
use orbit::OrbitCamera;
use support::{argument, flag};

const BAKE_ASSET: &str = "outdoor.dynlight";
/// Real time before `--frames` starts counting, so the lit pipelines have compiled.
const WARM_UP_SECONDS: f32 = 2.0;

fn main() -> AppExit {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(support::window("DynLight outdoors")),
        ..default()
    }))
    .add_plugins(DynamicLightingPlugin)
    .add_plugins((baking::plugin, capture::plugin, orbit::plugin))
    .insert_resource(BakeSettings {
        pixel_density: 20.0,
        ..default()
    })
    // The sky is the only fill light.
    .insert_resource(DynamicLightingSettings {
        ambient: Color::NONE,
        ..default()
    })
    .add_systems(Startup, (spawn, load_or_bake))
    .add_systems(Update, (apply_finished_bake, exit_after_frames));
    if !capture::active() {
        app.add_systems(Update, scene::animate);
    }
    app.run()
}

fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    scene::spawn(&mut commands, &mut meshes, &mut materials, &mut images);
    if let Some(transform) = support::fixed_camera() {
        commands.spawn((Camera3d::default(), transform));
        return;
    }
    let mut camera = OrbitCamera::new(Vec3::new(0.0, 1.0, -1.0), 0.55, 0.42, 24.0);
    camera.distance_range = (6.0, 60.0);
    camera.pitch_range = (0.05, 1.4);
    camera.drift = if capture::active() { 0.0 } else { 0.03 };
    commands.spawn((Camera3d::default(), camera));
}

fn load_or_bake(
    server: Res<AssetServer>,
    mut active: ResMut<ActiveBake>,
    mut pending: ResMut<PendingBake>,
) {
    if flag("--bake") {
        pending.queue(BAKE_ASSET, BakePreset::Full);
        return;
    }
    baking::load_existing(BAKE_ASSET, &server, &mut active);
    if active.0.is_none() {
        warn!("no bake yet: run with --bake; until then only the sun's realtime shadows show");
        support::status("Not baked: only the sun's realtime shadows show");
    }
}

fn apply_finished_bake(mut finished: MessageReader<BakeFinished>, mut active: ResMut<ActiveBake>) {
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
            }
            Err(error) => error!("bake failed: {error}"),
        }
    }
}

/// `--frames N`: exits after N frames, with a screenshot of one of the last if `--screenshot`.
fn exit_after_frames(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut frames: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(limit) = argument("--frames").and_then(|value| value.parse::<u32>().ok()) else {
        return;
    };
    if time.elapsed_secs() < WARM_UP_SECONDS {
        return;
    }
    *frames += 1;
    if let Some(path) = argument("--screenshot")
        && *frames == limit.saturating_sub(5).max(1)
    {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }
    if *frames >= limit {
        exit.write(AppExit::Success);
    }
}
