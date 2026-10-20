//! `--capture path.png`: saves one still, repeatable frame and exits, for the visual baselines in
//! `tests/gpu_baselines.rs`.
//!
//! The window is 1280×720 at a scale factor of 1, and virtual time stops at [`CAPTURE_SECONDS`],
//! so light effects, shimmer, the shader clock and anything animated hold still. The frame is
//! saved once a `--bake` has finished, the sky is built, the reflection probes are captured, no render pipeline has been compiling for
//! a while, and two screenshots in a row are identical.

// Each example uses a different subset.
#![allow(dead_code)]

use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::{PresentMode, PrimaryWindow, WindowResolution};
use bevy::winit::WinitSettings;
use dynlight::bake::BakeFinished;
use dynlight::environment::SkyStatus;
use dynlight::reflection_probes::ReflectionProbeStatus;

use crate::support;

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
/// Where virtual time stops. Not zero, so effects are part-way through their cycles.
const CAPTURE_SECONDS: f32 = 2.5;
/// Real time with no pipeline compiling before the first screenshot. Frame counts would be too
/// short a wait at uncapped frame rates.
const IDLE_SECONDS: f32 = 0.3;
/// Real time between the screenshots that must match.
const SECONDS_BETWEEN_SHOTS: f32 = 0.3;
const TIMEOUT_SECONDS: f32 = 120.0;

/// Whether this run captures a frame (`--capture path.png`).
pub fn active() -> bool {
    support::argument("--capture").is_some()
}

pub fn plugin(app: &mut App) {
    let Some(path) = support::argument("--capture") else {
        return;
    };
    track_pipelines(app);
    app.insert_resource(WinitSettings::continuous())
        .insert_resource(CaptureState {
            path,
            bake_pending: support::flag("--bake"),
            idle_since: None,
            last_shot_at: 0.0,
            in_flight: false,
            captured: None,
            previous: None,
        })
        .add_systems(Startup, (freeze_time, size_window))
        .add_systems(Last, (note_finished_bakes, step).chain());
}

pub use dynlight::reflection_probes::WaitingPipelines;

/// Adds [`WaitingPipelines`], once.
pub fn track_pipelines(app: &mut App) {
    WaitingPipelines::track(app);
}

#[derive(Resource)]
struct CaptureState {
    path: String,
    bake_pending: bool,
    /// Real time since when no pipeline has been compiling.
    idle_since: Option<f32>,
    last_shot_at: f32,
    in_flight: bool,
    captured: Option<Image>,
    previous: Option<Vec<u8>>,
}

fn freeze_time(mut time: ResMut<Time<Virtual>>) {
    time.pause();
    time.advance_to(Duration::from_secs_f32(CAPTURE_SECONDS));
}

fn size_window(mut window: Single<&mut Window, With<PrimaryWindow>>) {
    window.resolution = WindowResolution::new(WIDTH, HEIGHT).with_scale_factor_override(1.0);
    window.resizable = false;
    window.present_mode = PresentMode::AutoNoVsync;
}

fn note_finished_bakes(
    mut finished: MessageReader<BakeFinished>,
    mut state: ResMut<CaptureState>,
    mut exit: MessageWriter<AppExit>,
) {
    for bake in finished.read() {
        if let Err(error) = &bake.result {
            error!("capture: the bake failed: {error}");
            exit.write(AppExit::error());
        }
        state.bake_pending = false;
    }
}

fn step(
    mut commands: Commands,
    mut state: ResMut<CaptureState>,
    waiting: Res<WaitingPipelines>,
    (sky, probes): (Option<Res<SkyStatus>>, Option<Res<ReflectionProbeStatus>>),
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
) {
    if time.elapsed_secs() > TIMEOUT_SECONDS {
        error!("capture: no still frame after {TIMEOUT_SECONDS} s");
        exit.write(AppExit::error());
        return;
    }
    if let Some(image) = state.captured.take() {
        state.in_flight = false;
        if image.data.is_some() && state.previous == image.data {
            save(&image, &state.path, &time, &mut exit);
            return;
        }
        state.previous = image.data;
    }
    let now = time.elapsed_secs();
    let sky_building = sky.is_some_and(|sky| sky.building);
    let probes_pending = probes.is_some_and(|probes| probes.pending > 0);
    if state.bake_pending || sky_building || probes_pending || waiting.compiling() {
        state.idle_since = None;
        state.previous = None;
        return;
    }
    let idle_since = *state.idle_since.get_or_insert(now);
    let due = state.previous.is_none() || now - state.last_shot_at >= SECONDS_BETWEEN_SHOTS;
    if now - idle_since < IDLE_SECONDS || state.in_flight || !due {
        return;
    }
    state.in_flight = true;
    state.last_shot_at = now;
    commands.spawn(Screenshot::primary_window()).observe(
        |captured: On<ScreenshotCaptured>, mut state: ResMut<CaptureState>| {
            state.captured = Some(captured.image.clone());
        },
    );
}

fn save(image: &Image, path: &str, time: &Time<Real>, exit: &mut MessageWriter<AppExit>) {
    let saved = image
        .clone()
        .try_into_dynamic()
        .map_err(|error| error.to_string())
        .and_then(|frame| {
            frame
                .to_rgb8()
                .save(path)
                .map_err(|error| error.to_string())
        });
    match saved {
        Ok(()) => {
            info!("capture: saved {path} after {:.1} s", time.elapsed_secs());
            exit.write(AppExit::Success);
        }
        Err(error) => {
            error!("capture: could not save {path}: {error}");
            exit.write(AppExit::error());
        }
    }
}
