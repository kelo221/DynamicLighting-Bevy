//! Lighting test scene: every light type, every effect, shimmer, a cookie, fog volumes and
//! fluorescent area-light fixtures over a lit floor, wall and props, all drawn with
//! `DynLitMaterial` and no Bevy lights.
//!
//! Drive the cube with WASD or the arrow keys (Shift runs) and switch its lantern with Space;
//! drag to orbit and scroll to zoom. `--bake` (or the B key) bakes the floor, wall and crates;
//! the cube and the spheres are lit through the light BVH and distance cubes. V cycles the
//! debug views (`--debug-view lighting|light-count` starts in one).
//!
//! `cargo run --example lighting_test -- --frames 120` exits after 120 frames (smoke test);
//! add `--screenshot screenshots/lighting_test.png` to save the last frames as an image,
//! `--camera x,y,z,target_x,target_y,target_z` to hold the camera still, and `--gizmos` to
//! outline every light's radius.

mod bake;
#[path = "../support/baking.rs"]
mod baking;
mod debug;
mod fixtures;
#[path = "../support/orbit.rs"]
mod orbit;
mod player;
#[path = "../support/mod.rs"]
mod support;

use orbit::{OrbitCamera, OrbitFollow};
use player::Solid;
use support::argument;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use dynlight::prelude::*;

/// Frames left between requesting the screenshot and exiting; capture is asynchronous.
const SCREENSHOT_LEAD_FRAMES: u32 = 5;
/// Real time before `--frames` starts counting.
const WARM_UP_SECONDS: f32 = 2.0;

/// When `--frames` starts counting. Pipelines compile asynchronously; uncapped frame rates would
/// otherwise capture the clear color before the scene appears.
#[derive(Resource)]
struct WarmUp {
    ready_at: f32,
}

#[derive(Resource)]
struct ExitAfterFrames {
    frames: u32,
    screenshot: Option<String>,
}

fn main() -> AppExit {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(support::window("DynLight lighting test")),
        ..default()
    }))
    .add_plugins(DynamicLightingPlugin)
    .insert_resource(LightGizmos {
        enabled: support::args().any(|arg| arg == "--gizmos"),
    })
    .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
    .insert_resource(WarmUp {
        ready_at: WARM_UP_SECONDS,
    })
    .add_plugins((bake::plugin, debug::plugin, orbit::plugin, player::plugin))
    .add_systems(Startup, spawn_scene);
    // Browser bakes stay small and in memory; Preview skips bounce, preserving the scene.
    #[cfg(target_arch = "wasm32")]
    app.insert_resource(BakeSettings {
        pixel_density: 8.0,
        max_lightmap_size: 256,
        ..default()
    });
    if let Some(frames) = argument("--frames").and_then(|value| value.parse().ok()) {
        app.insert_resource(ExitAfterFrames {
            frames,
            screenshot: argument("--screenshot"),
        })
        .add_systems(Last, exit_after_frames);
    }
    app.run()
}

fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let player = player::spawn(
        &mut commands,
        &mut meshes,
        &mut materials,
        Vec2::new(0.0, 4.6),
    );
    let mut camera = commands.spawn((Camera3d::default(), FogPostProcess));
    match fixed_camera() {
        Some(transform) => camera.insert(transform),
        None => camera.insert((orbit_camera(), OrbitFollow(player))),
    };

    let floor = materials.add(Color::srgb(0.8, 0.8, 0.8));
    let wall = materials.add(Color::srgb(0.7, 0.72, 0.75));
    let prop = materials.add(Color::srgb(0.9, 0.85, 0.8));
    commands.spawn((
        Name::new("Floor"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(40.0, 40.0).subdivisions(40))),
        MeshMaterial3d(floor),
        BakeStatic,
    ));
    commands.spawn((
        Name::new("Wall"),
        Mesh3d(meshes.add(box_with_lightmap_uvs(Cuboid::new(40.0, 8.0, 0.5)))),
        MeshMaterial3d(wall),
        Transform::from_xyz(0.0, 4.0, -6.0),
        BakeStatic,
    ));
    let cube = meshes.add(box_with_lightmap_uvs(Cuboid::new(0.6, 0.6, 0.6)));
    let sphere = meshes.add(Sphere::new(0.35).mesh().uv(32, 16));
    for i in 0..8 {
        let x = i as f32 * 2.5 - 8.75;
        commands.spawn((
            Name::new(format!("Crate {i}")),
            Mesh3d(cube.clone()),
            MeshMaterial3d(prop.clone()),
            Transform::from_xyz(x + 0.7, 0.3, -2.4).with_rotation(Quat::from_rotation_y(0.6)),
            BakeStatic,
            Solid(0.4),
        ));
    }
    // Metallic shading: metals and dielectrics alternate, roughness grows to the right.
    for i in 0..15 {
        let x = i as f32 * 1.4 - 9.8;
        let material = materials.add(DynLitMaterial {
            base_color: Color::srgb(0.95, 0.8, 0.6),
            shading: ShadingModel::Metallic,
            metallic: if i % 2 == 0 { 1.0 } else { 0.0 },
            perceptual_roughness: 0.15 + i as f32 / 14.0 * 0.75,
            ..default()
        });
        commands.spawn((
            Mesh3d(sphere.clone()),
            MeshMaterial3d(material),
            Transform::from_xyz(x, 0.35, 2.6),
            Solid(0.35),
        ));
    }

    for (i, light_type) in LightType::ALL.into_iter().enumerate() {
        let hue = i as f32 / LightType::ALL.len() as f32 * 360.0;
        let mut light = commands.spawn((
            Name::new(format!("{light_type:?}")),
            DynamicLight {
                light_type,
                color: Color::hsl(hue, 0.7, 0.6),
                radius: 3.5,
                size: match light_type {
                    LightType::Rectangle => Vec2::new(0.8, 0.4),
                    LightType::Tube => Vec2::new(1.2, 0.0),
                    _ => Vec2::ZERO,
                },
                wave_frequency: 2.0,
                // One light bakes bounce lighting, which switches the bake to the bounce layout.
                illumination: if i == 0 {
                    Illumination::SingleBounce
                } else {
                    Illumination::Direct
                },
                // Strong enough to see in this open scene.
                bounce: LightBounce {
                    intensity: 3.0,
                    ..default()
                },
                ..default()
            },
            Transform::from_xyz(i as f32 * 2.5 - 8.75, 1.8, -3.0)
                .looking_to(Dir3::NEG_Y, Vec3::NEG_Z),
        ));
        if light_type == LightType::Spot {
            light.insert(LightCookie(images.add(cookie_image())));
        }
    }

    for (i, kind) in LightEffectKind::ALL.into_iter().enumerate() {
        commands.spawn((
            Name::new(format!("{kind:?}")),
            DynamicLight {
                color: Color::srgb(1.0, 0.75, 0.45),
                radius: 1.6,
                ..default()
            },
            LightEffect::new(kind),
            Transform::from_xyz(i as f32 * 1.4 - 9.8, 1.0, 1.5),
        ));
    }

    for (x, shimmer) in [(-4.0, Shimmer::Water), (4.0, Shimmer::Random)] {
        commands.spawn((
            Name::new(format!("{shimmer:?} shimmer")),
            DynamicLight {
                color: Color::srgb(0.4, 0.7, 1.0),
                radius: 4.0,
                shimmer,
                shimmer_scale: 4.0,
                ..default()
            },
            Transform::from_xyz(x, 1.5, 6.0),
        ));
    }

    fixtures::spawn(&mut commands, &mut meshes, &mut materials, prop);
}

/// A box whose faces get their own cell of a 3×2 UV1 atlas. Bevy's box UVs put every face on
/// the same square, and bounce lighting is baked into one lightmap per mesh, so faces must not
/// overlap there.
fn box_with_lightmap_uvs(cuboid: Cuboid) -> Mesh {
    const PADDING: f32 = 0.05;
    let mut mesh = cuboid.mesh().build();
    let Some(VertexAttributeValues::Float32x2(uv0)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
        return mesh;
    };
    // Four vertices per face.
    let uv1: Vec<[f32; 2]> = uv0
        .iter()
        .enumerate()
        .map(|(vertex, [u, v])| {
            let face = (vertex / 4) as f32;
            let cell = Vec2::new(face % 3.0, (face / 3.0).floor());
            let inner = Vec2::new(*u, *v) * (1.0 - 2.0 * PADDING) + PADDING;
            ((cell + inner) / Vec2::new(3.0, 2.0)).to_array()
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    mesh
}

/// A 64² ring pattern for the spot light's cookie.
fn cookie_image() -> Image {
    let size = 64u32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let d = Vec2::new(x as f32 - 31.5, y as f32 - 31.5).length() / 32.0;
            let value = if (d * 4.0).fract() < 0.5 { 255 } else { 40 };
            data.extend_from_slice(&[value, value, value, 255]);
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::all(),
    )
}

/// The camera from `--camera x,y,z,target_x,target_y,target_z`.
fn fixed_camera() -> Option<Transform> {
    let numbers: Vec<f32> = argument("--camera")?
        .split(',')
        .filter_map(|n| n.trim().parse().ok())
        .collect();
    let [x, y, z, tx, ty, tz] = numbers[..] else {
        return None;
    };
    Some(Transform::from_xyz(x, y, z).looking_at(Vec3::new(tx, ty, tz), Vec3::Y))
}

/// Orbits the cube from the front: the wall hides everything from behind.
fn orbit_camera() -> OrbitCamera {
    let mut orbit = OrbitCamera::new(Vec3::new(0.0, 0.5, 0.0), -0.3, 0.43, 15.0);
    orbit.yaw_range = Some((-1.2, 1.2));
    orbit.pitch_range = (0.15, 1.3);
    orbit.distance_range = (4.0, 26.0);
    orbit.drift = 0.12;
    orbit
}

fn exit_after_frames(
    mut commands: Commands,
    mut frames: Local<u32>,
    limit: Res<ExitAfterFrames>,
    time: Res<Time<Real>>,
    warm_up: Res<WarmUp>,
    mut exit: MessageWriter<AppExit>,
) {
    if time.elapsed_secs() < warm_up.ready_at {
        return;
    }
    *frames += 1;
    if let Some(path) = &limit.screenshot
        && *frames == limit.frames.saturating_sub(SCREENSHOT_LEAD_FRAMES).max(1)
    {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
    }
    if *frames >= limit.frames {
        exit.write(AppExit::Success);
    }
}
