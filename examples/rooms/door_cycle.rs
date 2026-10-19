//! `--door-cycle DIR`: shuts a door in front of a still camera, opens it again, and logs what
//! changed, for `tests/gpu_examples.rs`. A shadowed probe light in the corridor shines through
//! the middle north door onto the floor of the room behind it, and the lamp of the room across
//! the corridor is seen only through the doors.
//!
//! Each phase (open, shut, open again) waits until no pipeline is compiling and no shadow face
//! has rendered for a while, and two screenshots in a row match, saves `open.png`, `closed.png` or `reopened.png` in DIR, and
//! moves on. The `rooms door cycle:` line gives the mean luminance of the floor just inside the
//! door in each phase, how many pixels differ between the open and reopened frames, the shadow
//! faces rendered after each change, and whether the probe and the lamp across the corridor
//! were packed and had a shadow cubemap in each phase.

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::{PresentMode, PrimaryWindow, WindowResolution};
use dynlight::light::PackedLights;
use dynlight::prelude::*;
use dynlight::realtime_shadows::{ShadowChoice, ShadowCubemapStats};

use crate::capture::{WaitingPipelines, track_pipelines};
use crate::{doors, support};

const DOOR: &str = "North room 1 door";
const PROBE: &str = "Probe light";
/// Seen only through the door from the room behind it, so shutting it culls this lamp.
const ACROSS: &str = "South room 1 lamp";
const PHASES: [&str; 3] = ["open", "closed", "reopened"];
/// Real time before the first phase starts, so pipelines have compiled.
const WARM_UP_SECONDS: f32 = 3.0;
/// Frames in a row without a shadow face rendered before a phase may end.
const QUIET_FRAMES: u32 = 30;
const TIMEOUT_SECONDS: f32 = 120.0;

#[derive(Resource)]
struct DoorCycle {
    dir: PathBuf,
    phase: usize,
    quiet: u32,
    faces: [u32; 3],
    in_flight: bool,
    captured: Option<Image>,
    previous: Option<Vec<u8>>,
    /// Each phase's frame: width and RGB bytes.
    frames: Vec<(u32, Vec<u8>)>,
    /// Each phase: the probe and the lamp across, packed and with a cube.
    lights: Vec<[(bool, bool); 2]>,
}

pub fn plugin(app: &mut App) {
    let Some(dir) = support::argument("--door-cycle") else {
        return;
    };
    track_pipelines(app);
    app.insert_resource(DoorCycle {
        dir: dir.into(),
        phase: 0,
        quiet: 0,
        faces: [0; 3],
        in_flight: false,
        captured: None,
        previous: None,
        frames: Vec::new(),
        lights: Vec::new(),
    })
    .add_systems(Startup, setup)
    .add_systems(Last, step);
}

fn setup(
    mut commands: Commands,
    mut time: ResMut<Time<Virtual>>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
) {
    time.pause();
    time.advance_to(Duration::from_secs_f32(2.5));
    window.resolution = WindowResolution::new(1280, 720).with_scale_factor_override(1.0);
    window.present_mode = PresentMode::AutoNoVsync;
    commands.spawn((
        Name::new(PROBE),
        DynamicLight {
            color: Color::srgb(1.0, 0.9, 0.75),
            radius: 4.0,
            intensity: 2.0,
            realtime: true,
            shadows: ShadowMode::Realtime,
            ..DynamicLight::point()
        },
        Transform::from_xyz(13.5, 1.4, -0.8),
    ));
}

#[allow(clippy::too_many_arguments)]
fn step(
    mut commands: Commands,
    mut cycle: ResMut<DoorCycle>,
    (time, waiting): (Res<Time<Real>>, Res<WaitingPipelines>),
    stats: Res<ShadowCubemapStats>,
    (packed, choice): (Res<PackedLights>, Res<ShadowChoice>),
    names: Query<(Entity, &Name)>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    if now > TIMEOUT_SECONDS {
        error!(
            "rooms door cycle: phase {} never settled",
            PHASES[cycle.phase]
        );
        exit.write(AppExit::error());
        return;
    }
    if now < WARM_UP_SECONDS {
        return;
    }
    let cycle = &mut *cycle;
    cycle.faces[cycle.phase] += stats.faces_rendered;
    if stats.faces_rendered > 0 || waiting.compiling() {
        cycle.quiet = 0;
        cycle.previous = None;
    } else {
        cycle.quiet += 1;
    }
    let named = |name: &str| {
        names
            .iter()
            .find(|(_, entity_name)| entity_name.as_str() == name)
            .map(|(entity, _)| entity)
    };
    if let Some(image) = cycle.captured.take() {
        cycle.in_flight = false;
        if image.data.is_some() && cycle.previous == image.data {
            let state = |name| {
                let light = named(name);
                let packed = light.is_some_and(|light| packed.realtime.contains(&light));
                (
                    packed,
                    light.is_some_and(|light| choice.lights.contains(&light)),
                )
            };
            cycle.lights.push([state(PROBE), state(ACROSS)]);
            let Some(frame) = save(&image, &cycle.dir, PHASES[cycle.phase]) else {
                exit.write(AppExit::error());
                return;
            };
            cycle.frames.push(frame);
            cycle.phase += 1;
            cycle.quiet = 0;
            cycle.previous = None;
            let Some(door) = named(DOOR) else {
                error!("rooms door cycle: no {DOOR}");
                exit.write(AppExit::error());
                return;
            };
            match cycle.phase {
                1 | 2 => doors::shut(&mut commands, door, cycle.phase == 1),
                _ => {
                    let (camera, transform) = *camera;
                    info!("{}", report(cycle, camera, transform));
                    exit.write(AppExit::Success);
                }
            }
            return;
        }
        cycle.previous = image.data;
    }
    if cycle.quiet < QUIET_FRAMES || cycle.in_flight {
        return;
    }
    cycle.in_flight = true;
    commands.spawn(Screenshot::primary_window()).observe(
        |captured: On<ScreenshotCaptured>, mut cycle: ResMut<DoorCycle>| {
            cycle.captured = Some(captured.image.clone());
        },
    );
}

/// Saves the frame as `DIR/<phase>.png` and returns its width and RGB bytes.
fn save(image: &Image, dir: &std::path::Path, phase: &str) -> Option<(u32, Vec<u8>)> {
    let rgb = image.clone().try_into_dynamic().ok()?.to_rgb8();
    let path = dir.join(format!("{phase}.png"));
    if let Err(error) = rgb.save(&path) {
        error!(
            "rooms door cycle: could not save {}: {error}",
            path.display()
        );
        return None;
    }
    Some((rgb.width(), rgb.into_raw()))
}

/// The mean luminance of the floor just inside the door, from points on it.
fn floor_by_the_door(frame: &(u32, Vec<u8>), camera: &Camera, transform: &GlobalTransform) -> f32 {
    let (width, rgb) = frame;
    let height = (rgb.len() / 3) as u32 / width;
    let mut sum = 0.0;
    let mut count = 0;
    for i in 0..13 {
        for j in 0..6 {
            let point = Vec3::new(12.9 + 0.1 * i as f32, 0.001, 1.65 + 0.2 * j as f32);
            let Ok(pixel) = camera.world_to_viewport(transform, point) else {
                continue;
            };
            let (x, y) = (pixel.x as u32, pixel.y as u32);
            if x >= *width || y >= height {
                continue;
            }
            let at = 3 * (y * width + x) as usize;
            let [r, g, b] = [rgb[at], rgb[at + 1], rgb[at + 2]].map(|value| value as f32 / 255.0);
            sum += 0.2126 * r + 0.7152 * g + 0.0722 * b;
            count += 1;
        }
    }
    if count == 0 {
        f32::NAN
    } else {
        sum / count as f32
    }
}

fn report(cycle: &DoorCycle, camera: &Camera, transform: &GlobalTransform) -> String {
    let floor = cycle
        .frames
        .iter()
        .map(|frame| floor_by_the_door(frame, camera, transform));
    let floor: Vec<String> = floor.map(|value| format!("{value:.4}")).collect();
    let changed = cycle.frames[0]
        .1
        .chunks(3)
        .zip(cycle.frames[2].1.chunks(3))
        .filter(|(a, b)| a != b)
        .count();
    let phases = |light: usize, cube: bool| {
        let states: Vec<&str> = cycle
            .lights
            .iter()
            .map(|lights| {
                let (packed, has_cube) = lights[light];
                let on = if cube { has_cube } else { packed };
                if on { "1" } else { "0" }
            })
            .collect();
        states.join("/")
    };
    format!(
        "rooms door cycle: floor by the door {}; pixels changed on reopening {changed}; faces \
         after closing {}; faces after reopening {}; probe packed {}; probe cube {}; lamp \
         across packed {}; lamp across cube {}",
        floor.join("/"),
        cycle.faces[1],
        cycle.faces[2],
        phases(0, false),
        phases(0, true),
        phases(1, false),
        phases(1, true),
    )
}
