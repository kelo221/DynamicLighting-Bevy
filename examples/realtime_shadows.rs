//! Realtime shadow cubemaps: a moving point light and a spot light with `ShadowMode::Realtime`
//! cast shadows of spinning crates and pillars onto the floor. Nothing is baked.
//!
//! `cargo run --example realtime_shadows -- --frames 60 --screenshot screenshots/shadows.png`
//! exits after 60 frames and saves the last ones as an image.

use core::f32::consts::TAU;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use dynlight::prelude::*;

/// Real time before `--frames` starts counting, so pipelines have compiled.
const WARM_UP_SECONDS: f32 = 2.0;

#[derive(Component)]
struct Orbit {
    radius: f32,
    height: f32,
    speed: f32,
    phase: f32,
}

#[derive(Component)]
struct Spin;

fn main() -> AppExit {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins)
        .add_plugins(DynamicLightingPlugin)
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
        .add_systems(Startup, spawn_scene)
        .add_systems(Update, (orbit, spin));
    if let Some(frames) = argument("--frames").and_then(|value| value.parse::<u32>().ok()) {
        let screenshot = argument("--screenshot");
        app.add_systems(
            Last,
            move |mut commands: Commands,
                  mut counted: Local<u32>,
                  time: Res<Time<Real>>,
                  mut exit: MessageWriter<AppExit>| {
                if time.elapsed_secs() < WARM_UP_SECONDS {
                    return;
                }
                *counted += 1;
                if let Some(path) = &screenshot
                    && *counted == frames.saturating_sub(5).max(1)
                {
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(path.clone()));
                }
                if *counted >= frames {
                    exit.write(AppExit::Success);
                }
            },
        );
    }
    app.run()
}

fn argument(name: &str) -> Option<String> {
    let mut args = std::env::args().skip_while(|arg| arg != name);
    args.next()?;
    args.next()
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
        commands.spawn((
            Mesh3d(crate_mesh.clone()),
            MeshMaterial3d(wood.clone()),
            Transform::default(),
            Orbit {
                radius: 2.2,
                height: 0.8,
                speed: 0.3,
                phase: i as f32 / 4.0 * TAU,
            },
            Spin,
        ));
    }

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
        Transform::default(),
        Orbit {
            radius: 1.0,
            height: 2.6,
            speed: -0.2,
            phase: 0.0,
        },
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
}

fn orbit(time: Res<Time>, mut movers: Query<(&mut Transform, &Orbit)>) {
    for (mut transform, orbit) in &mut movers {
        let angle = orbit.phase + time.elapsed_secs() * orbit.speed * TAU;
        transform.translation = Vec3::new(
            angle.cos() * orbit.radius,
            orbit.height,
            angle.sin() * orbit.radius,
        );
    }
}

fn spin(time: Res<Time>, mut crates: Query<&mut Transform, With<Spin>>) {
    for mut transform in &mut crates {
        transform.rotation = Quat::from_rotation_y(time.elapsed_secs() * 0.7)
            * Quat::from_rotation_x(time.elapsed_secs() * 0.4);
    }
}
