//! Rooms and portals: six rooms off a corridor, each with a shadowed lamp and two other lights,
//! one with a cone of fog, and a window onto a yard with its own shadowed light. Every door and
//! the window is a portal, so the lights, shadow cubemaps and fog the camera can't see through
//! them are left out. Walls are never taken to stop light: a light is only left out when its
//! whole sphere misses everything the camera sees.
//!
//! P switches portal culling (`--no-portals` starts without it) and G the room and light
//! gizmos (`--gizmos`). The title bar shows what is in view and what is drawn, and once a
//! second the log says what portal culling did:
//! `dynlight portals: rooms N, visits N, unbounded N, realtime lights N, cubes N, cubes
//! assigned N, faces N, cluster entries N, fog N` (cubes assigned and faces over that second,
//! the rest this frame).
//!
//! - `--path` flies through the rooms; with `--frames` it moves a fixed step per frame, so
//!   every run sees the same frames.
//! - `--still` stops the lamps bobbing, so cached shadow faces stay cached.
//! - `--grid N` builds `N × N` rooms instead, three lights and a cube each; `--budgets` raises
//!   the realtime light and shadow cubemap budgets to fit every light.
//! - `--frames N` exits after measuring N frames and logs the mean frame time and stats;
//!   `--screenshot path.png` also saves an image. `--capture path.png` saves a still frame for
//!   the visual baselines, `--camera x,y,z,tx,ty,tz` holds the camera somewhere else.

#[path = "../support/capture.rs"]
mod capture;
mod grid;
mod layout;
mod path;
#[path = "../support/mod.rs"]
mod support;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{PresentMode, PrimaryWindow};
use dynlight::fog::FogVolumes;
use dynlight::light::PackedLights;
use dynlight::light_clusters::RealtimeClusters;
use dynlight::portals::{PortalStats, RoomGraph, RoomView};
use dynlight::prelude::*;
use dynlight::realtime_shadows::ShadowCubemapStats;
use support::{argument, flag};

/// Real time before `--frames` starts counting, so pipelines have compiled.
const WARM_UP_SECONDS: f32 = 3.0;

#[derive(Resource)]
struct Options {
    frames: Option<u32>,
    screenshot: Option<String>,
    grid: Option<u32>,
}

fn main() -> AppExit {
    let options = Options {
        frames: argument("--frames").and_then(|value| value.parse().ok()),
        screenshot: argument("--screenshot"),
        grid: argument("--grid").and_then(|value| value.parse().ok()),
    };
    let mut window = support::window("DynLight rooms");
    if options.frames.is_some() {
        // Measure the real cost, not the display's refresh rate.
        window.present_mode = PresentMode::AutoNoVsync;
    }
    let mut settings = DynamicLightingSettings::default();
    if flag("--budgets") {
        let (lights, cubes) = layout::counts(options.grid);
        settings.realtime_light_budget = settings.realtime_light_budget.max(lights);
        settings.shadow_cubemap_budget = settings.shadow_cubemap_budget.max(cubes);
    }
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(window),
        ..default()
    }))
    .add_plugins((DynamicLightingPlugin, capture::plugin))
    .insert_resource(settings)
    .insert_resource(PortalSettings {
        enabled: !flag("--no-portals"),
        ..default()
    })
    .insert_resource(LightGizmos {
        enabled: flag("--gizmos"),
    })
    .insert_resource(ClearColor(Color::srgb(0.03, 0.04, 0.07)))
    .add_systems(Startup, spawn)
    .add_systems(Update, toggle)
    .add_systems(Last, report);
    if !flag("--still") {
        app.add_systems(Update, path::bob);
    }
    if flag("--path") && !capture::active() {
        let mut path = path::Path::new(options.grid);
        if options.frames.is_some() {
            path.fixed_after = Some(WARM_UP_SECONDS);
        }
        app.insert_resource(path).add_systems(Update, path::fly);
    }
    app.insert_resource(options);
    app.run()
}

fn spawn(
    mut commands: Commands,
    options: Res<Options>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    let kit = layout::Kit::new(&mut meshes, &mut materials);
    match options.grid {
        Some(n) => grid::spawn(&mut commands, &kit, n.max(1)),
        None => layout::corridor(&mut commands, &kit),
    }
    let view = support::fixed_camera().unwrap_or_else(|| layout::view(options.grid));
    commands.spawn((Camera3d::default(), FogPostProcess, view));
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut portals: ResMut<PortalSettings>,
    mut gizmos: ResMut<LightGizmos>,
) {
    if keys.just_pressed(KeyCode::KeyP) {
        portals.enabled = !portals.enabled;
    }
    if keys.just_pressed(KeyCode::KeyG) {
        gizmos.enabled = !gizmos.enabled;
    }
}

/// This frame's numbers.
#[derive(Clone, Copy, Default)]
struct Frame {
    visits: u32,
    lights: u32,
    cubes: u32,
    assigned: u32,
    faces: u32,
    entries: u32,
    fog: u32,
}

#[derive(Default)]
struct Report {
    last_log: f32,
    last_title: f32,
    /// Cubes assigned and faces rendered since the last log line.
    since_log: (u32, u32),
    frames: u32,
    seconds: f64,
    sums: [f64; 7],
}

#[allow(clippy::too_many_arguments)]
fn report(
    mut commands: Commands,
    options: Res<Options>,
    time: Res<Time<Real>>,
    (portals, settings, graph): (Res<PortalStats>, Res<PortalSettings>, Res<RoomGraph>),
    (packed, shadows): (Res<PackedLights>, Res<ShadowCubemapStats>),
    (clusters, fog): (Res<RealtimeClusters>, Res<FogVolumes>),
    views: Query<&RoomView, With<Camera3d>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut state: Local<Report>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    let frame = Frame {
        visits: portals.visits,
        lights: packed.realtime.len() as u32,
        cubes: shadows.lights,
        assigned: shadows.cubes_assigned,
        faces: shadows.faces_rendered,
        entries: clusters.lists.entries as u32,
        fog: fog.0.len() as u32,
    };
    state.since_log.0 += frame.assigned;
    state.since_log.1 += frame.faces;
    if now - state.last_log >= 1.0 {
        state.last_log = now;
        let (assigned, faces) = std::mem::take(&mut state.since_log);
        info!(
            "dynlight portals: rooms {}, visits {}, unbounded {}, realtime lights {}, cubes {}, \
             cubes assigned {assigned}, faces {faces}, cluster entries {}, fog {}",
            portals.rooms,
            frame.visits,
            portals.unbounded_views,
            frame.lights,
            frame.cubes,
            frame.entries,
            frame.fog
        );
    }
    if now - state.last_title >= 0.5 {
        state.last_title = now;
        let (lights, cubes) = layout::counts(options.grid);
        let seen = views
            .iter()
            .next()
            .and_then(|view| view.bounded(graph.generation()))
            .map(|traced| {
                (1..=graph.rooms.len())
                    .filter(|&node| traced.visits(node))
                    .count()
            });
        let culling = match (settings.enabled, seen) {
            (false, _) => "portals off (P)".to_string(),
            (true, Some(seen)) => format!(
                "portals on (P), {seen} of {} rooms in view",
                graph.rooms.len()
            ),
            (true, None) => "portals on (P), not culling".to_string(),
        };
        let title = format!(
            "DynLight rooms · {culling} · {} of {lights} lights · {} of {cubes} shadow cubes · {} fog volumes",
            frame.lights, frame.cubes, frame.fog
        );
        support::status(&title);
        if let Ok(mut window) = windows.single_mut() {
            window.title = title;
        }
    }
    measure(
        &mut commands,
        &options,
        &time,
        frame,
        &mut state,
        &mut exit,
        settings.enabled,
    );
}

/// With `--frames`: counts frames after the warm-up, saves the screenshot near the end and logs
/// the means before exiting.
fn measure(
    commands: &mut Commands,
    options: &Options,
    time: &Time<Real>,
    frame: Frame,
    state: &mut Report,
    exit: &mut MessageWriter<AppExit>,
    portals: bool,
) {
    let Some(frames) = options.frames else {
        return;
    };
    if time.elapsed_secs() < WARM_UP_SECONDS {
        return;
    }
    state.frames += 1;
    state.seconds += time.delta_secs_f64();
    let values = [
        frame.visits,
        frame.lights,
        frame.cubes,
        frame.assigned,
        frame.faces,
        frame.entries,
        frame.fog,
    ];
    for (sum, value) in state.sums.iter_mut().zip(values) {
        *sum += f64::from(value);
    }
    if let Some(path) = &options.screenshot
        && state.frames == frames.saturating_sub(5).max(1)
    {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
    }
    if state.frames >= frames {
        let count = f64::from(state.frames);
        let [visits, lights, cubes, assigned, faces, entries, fog] =
            state.sums.map(|sum| sum / count);
        info!(
            "rooms: portals {}, mean {:.3} ms per frame over {} frames; per frame: visits {visits:.1}, \
             realtime lights {lights:.2}, cubes {cubes:.2}, cubes assigned {assigned:.3}, faces \
             {faces:.2}, cluster entries {entries:.0}, fog {fog:.2}",
            if portals { "on" } else { "off" },
            state.seconds * 1000.0 / count,
            state.frames
        );
        exit.write(AppExit::Success);
    }
}
