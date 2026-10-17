//! Realtime shadow cubemaps: a moving point light and a spot light with `ShadowMode::Realtime`
//! cast shadows of spinning crates and pillars onto the floor. Nothing is baked.
//!
//! Cubemaps are cached: the orbiting lamp's cube renders every frame because the lamp moves,
//! while the still spot only renders the faces that see a moving crate.
//!
//! `cargo run --example realtime_shadows -- --frames 60 --screenshot screenshots/shadows.png`
//! exits after 60 frames, saves the last ones as an image and logs the mean frame time (VSync
//! off) and cube faces rendered per frame. `--still` stops everything, so no face needs
//! rendering.

#[path = "support/mod.rs"]
mod support;

use core::f32::consts::TAU;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::PresentMode;
use dynlight::prelude::*;
use dynlight::realtime_shadows::ShadowCubemapStats;
use support::{argument, flag};

/// Real time before `--frames` starts counting, so pipelines have compiled and still cubes have
/// settled (about 0.3 s on a desktop GPU).
const WARM_UP_SECONDS: f32 = 1.0;
/// Faces of the two cubes.
const ALL_FACES: u32 = 12;

#[derive(Component)]
struct Orbit {
    radius: f32,
    height: f32,
    speed: f32,
    phase: f32,
}

#[derive(Component)]
struct Spin;

#[derive(Resource)]
struct Options {
    frames: Option<u32>,
    screenshot: Option<String>,
}

#[derive(Default)]
struct Measurement {
    frames: u32,
    seconds: f64,
    faces: u64,
    last_log: f32,
}

fn main() -> AppExit {
    let options = Options {
        frames: argument("--frames").and_then(|value| value.parse().ok()),
        screenshot: argument("--screenshot"),
    };
    let mut window = support::window("DynLight realtime shadows");
    if options.frames.is_some() {
        // Measure the real cost, not the display's refresh rate.
        window.present_mode = PresentMode::AutoNoVsync;
    }
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(window),
        ..default()
    }))
    .add_plugins(DynamicLightingPlugin)
    .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
    .insert_resource(options)
    .add_systems(Startup, spawn_scene)
    .add_systems(Last, report);
    if !flag("--still") {
        app.add_systems(Update, (orbit, spin));
    }
    app.run()
}

fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 9.0, 12.0).looking_at(Vec3::new(0.0, 0.0, 0.5), Vec3::Y),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(30.0, 30.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.8, 0.8, 0.8))),
    ));

    let pillar = meshes.add(Cylinder::new(0.3, 3.0));
    let stone = materials.add(Color::srgb(0.7, 0.7, 0.75));
    for i in 0..6 {
        let angle = i as f32 / 6.0 * TAU;
        commands.spawn((
            Mesh3d(pillar.clone()),
            MeshMaterial3d(stone.clone()),
            Transform::from_xyz(angle.cos() * 4.5, 1.5, angle.sin() * 4.5),
        ));
    }

    let crate_mesh = meshes.add(Cuboid::new(0.8, 0.8, 0.8));
    let wood = materials.add(Color::srgb(0.85, 0.6, 0.35));
    for i in 0..4 {
        let orbit = Orbit {
            radius: 2.2,
            height: 0.8,
            speed: 0.3,
            phase: i as f32 / 4.0 * TAU,
        };
        commands.spawn((
            Mesh3d(crate_mesh.clone()),
            MeshMaterial3d(wood.clone()),
            orbit.transform(0.0),
            orbit,
            Spin,
        ));
    }

    let lamp = Orbit {
        radius: 1.0,
        height: 2.6,
        speed: -0.2,
        phase: 0.0,
    };
    commands.spawn((
        Name::new("Orbiting lamp"),
        DynamicLight {
            color: Color::srgb(1.0, 0.85, 0.6),
            radius: 9.0,
            intensity: 2.0,
            realtime: true,
            shadows: ShadowMode::Realtime,
            ..default()
        },
        lamp.transform(0.0),
        lamp,
    ));
    commands.spawn((
        Name::new("Spot"),
        DynamicLight {
            light_type: LightType::Spot,
            color: Color::srgb(0.5, 0.7, 1.0),
            radius: 12.0,
            intensity: 1.5,
            inner_cutoff: 20.0,
            outer_cutoff: 30.0,
            realtime: true,
            shadows: ShadowMode::Realtime,
            ..default()
        },
        Transform::from_xyz(-6.0, 6.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    support::status("Two realtime lights with shadow cubemaps; nothing is baked");
}

impl Orbit {
    fn transform(&self, seconds: f32) -> Transform {
        let angle = self.phase + seconds * self.speed * TAU;
        Transform::from_xyz(
            angle.cos() * self.radius,
            self.height,
            angle.sin() * self.radius,
        )
    }
}

fn orbit(time: Res<Time>, mut movers: Query<(&mut Transform, &Orbit)>) {
    for (mut transform, orbit) in &mut movers {
        transform.translation = orbit.transform(time.elapsed_secs()).translation;
    }
}

fn spin(time: Res<Time>, mut crates: Query<&mut Transform, With<Spin>>) {
    for mut transform in &mut crates {
        transform.rotation = Quat::from_rotation_y(time.elapsed_secs() * 0.7)
            * Quat::from_rotation_x(time.elapsed_secs() * 0.4);
    }
}

/// Logs the faces rendered per frame every second and, with `--frames`, the mean frame time.
fn report(
    mut commands: Commands,
    options: Res<Options>,
    time: Res<Time<Real>>,
    stats: Res<ShadowCubemapStats>,
    mut measurement: Local<Measurement>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    if now < WARM_UP_SECONDS {
        return;
    }
    measurement.frames += 1;
    measurement.seconds += time.delta_secs_f64();
    measurement.faces += u64::from(stats.faces_rendered);
    if now - measurement.last_log > 1.0 {
        measurement.last_log = now;
        support::status(&format!(
            "Two realtime lights with shadow cubemaps; {} of {ALL_FACES} cube faces rendered \
             this frame, the others are cached",
            stats.faces_rendered
        ));
    }
    let Some(frames) = options.frames else {
        return;
    };
    if let Some(path) = &options.screenshot
        && measurement.frames == frames.saturating_sub(5).max(1)
    {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
    }
    if measurement.frames >= frames {
        let count = f64::from(measurement.frames);
        info!(
            "realtime shadows: mean {:.3} ms per frame, {:.2} of {ALL_FACES} cube faces \
             rendered per frame, over {} frames",
            measurement.seconds * 1000.0 / count,
            measurement.faces as f64 / count,
            measurement.frames
        );
        exit.write(AppExit::Success);
    }
}
