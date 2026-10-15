//! A Feathers inspector for one `DynamicLight`: change its type, effect, color, intensity, radius
//! and cone live, switch it between baked and realtime, and bake the room with a button.
//!
//! `cargo run --example inspector --features inspector`. The UI needs Bevy's Feathers widgets,
//! which the `inspector` feature enables for this example only. `--bake` bakes at startup,
//! `--frames N` exits after N frames and `--screenshot path.png` saves the last one.

#[path = "../support/baking.rs"]
mod baking;
mod panel;
#[path = "../support/mod.rs"]
mod support;

use baking::PendingBake;
use support::argument;

use bevy::feathers::FeathersPlugins;
use bevy::feathers::dark_theme::create_dark_theme;
use bevy::feathers::theme::UiTheme;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use dynlight::bake::{BakeFinished, BakePreset};
use dynlight::baked_lighting::ActiveBake;
use dynlight::prelude::*;

const BAKE_ASSET: &str = "inspector.dynlight";
/// Real time before `--frames` starts counting, so pipelines have compiled.
const WARM_UP_SECONDS: f32 = 2.0;

/// The light the panel edits.
#[derive(Component, Clone, Default)]
struct Inspected;

fn main() -> AppExit {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins.set(WindowPlugin {
            primary_window: Some(support::window("DynLight inspector")),
            ..default()
        }),
        FeathersPlugins,
        DynamicLightingPlugin,
    ))
    .insert_resource(UiTheme(create_dark_theme()))
    .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
    .insert_resource(BakeSettings {
        pixel_density: if cfg!(target_arch = "wasm32") {
            8.0
        } else {
            32.0
        },
        max_lightmap_size: if cfg!(target_arch = "wasm32") {
            256
        } else {
            2048
        },
        ..default()
    })
    .add_plugins((baking::plugin, panel::plugin))
    .add_systems(Startup, (spawn_room, load_bake))
    .add_systems(Update, apply_finished_bake);
    if let Some(frames) = argument("--frames").and_then(|value| value.parse::<u32>().ok()) {
        let screenshot = argument("--screenshot");
        app.add_systems(
            Last,
            move |mut commands: Commands,
                  time: Res<Time<Real>>,
                  mut counted: Local<u32>,
                  mut exit: MessageWriter<AppExit>| {
                if time.elapsed_secs() < WARM_UP_SECONDS {
                    return;
                }
                *counted += 1;
                if *counted == frames
                    && let Some(path) = &screenshot
                {
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(path.clone()));
                } else if *counted == frames + 5 {
                    exit.write(AppExit::Success);
                }
            },
        );
    }
    app.run()
}

fn spawn_room(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    let with_lightmap_uvs = |mut mesh: Mesh| {
        let uv = mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().clone();
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv);
        mesh
    };
    let floor = meshes.add(with_lightmap_uvs(
        Plane3d::default().mesh().size(12.0, 12.0).build(),
    ));
    commands.spawn((
        Name::new("Floor"),
        Mesh3d(floor),
        MeshMaterial3d(materials.add(Color::srgb(0.75, 0.72, 0.68))),
        BakeStatic,
    ));
    let wall = meshes.add(with_lightmap_uvs(
        Plane3d::new(Vec3::Z, Vec2::new(6.0, 2.5)).mesh().build(),
    ));
    commands.spawn((
        Name::new("Back wall"),
        Mesh3d(wall),
        MeshMaterial3d(materials.add(Color::srgb(0.55, 0.6, 0.7))),
        Transform::from_xyz(0.0, 2.5, -4.0),
        BakeStatic,
    ));
    let pillar = meshes.add(with_lightmap_uvs(Cylinder::new(0.35, 3.0).mesh().build()));
    let stone = materials.add(Color::srgb(0.8, 0.78, 0.74));
    for (i, x) in [-2.5f32, 0.0, 2.5].into_iter().enumerate() {
        commands.spawn((
            Name::new(format!("Pillar {i}")),
            Mesh3d(pillar.clone()),
            MeshMaterial3d(stone.clone()),
            Transform::from_xyz(x, 1.5, -1.0 + x.abs() * 0.4),
            BakeStatic,
        ));
    }

    commands.spawn((
        Name::new("Inspected light"),
        Inspected,
        DynamicLight {
            color: Color::hsl(35.0, 0.7, 0.8),
            radius: 9.0,
            intensity: 2.5,
            outer_cutoff: 40.0,
            inner_cutoff: 34.0,
            size: Vec2::new(1.0, 0.5),
            ..DynamicLight::spot()
        },
        Transform::from_xyz(0.5, 4.5, 3.5).looking_at(Vec3::new(0.0, 0.0, -1.0), Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(3.5, 5.0, 9.0).looking_at(Vec3::new(-1.0, 1.0, -1.0), Vec3::Y),
    ));
}

fn load_bake(
    server: Res<AssetServer>,
    mut active: ResMut<ActiveBake>,
    mut pending: ResMut<PendingBake>,
) {
    // Browsers have no saved bake to load, so they start with a quick one.
    if cfg!(target_arch = "wasm32") || support::flag("--bake") {
        request_bake(&mut pending);
    } else {
        baking::load_existing(BAKE_ASSET, &server, &mut active);
        support::status("Ready: edit the light, then Bake (Preview skips bounce)");
    }
}

fn request_bake(pending: &mut PendingBake) {
    // Queue rather than send here: the browser must paint the status before the bake blocks.
    pending.queue(BAKE_ASSET, BakePreset::Preview);
}

fn apply_finished_bake(mut finished: MessageReader<BakeFinished>, mut active: ResMut<ActiveBake>) {
    for bake in finished.read() {
        match &bake.result {
            Ok(report) => {
                info!("baked in {} ms", report.milliseconds);
                active.0 = Some(report.handle.clone());
            }
            Err(error) => error!("bake failed: {error}"),
        }
    }
}
