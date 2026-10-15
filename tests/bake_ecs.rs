//! A `BakeRequest` in a headless app: snapshot, bake on the async compute pool, write the
//! `.dynlight` file, then load it back through the asset server.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy::mesh::MeshTag;
use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use dynlight::bake::mirror::TriangleView;
use dynlight::bake::{
    BakeFailure, BakeFinished, BakePreset, BakeRequest, BakeStatus, BakedScene, DynLightBake,
    SnapshotError, decode,
};
use dynlight::baked_lighting::{ActiveBake, AppliedBake, has_triangle_ids};
use dynlight::light::{BakedLightSlots, PackedLights};
use dynlight::prelude::*;
use dynlight::shared::{ResolvedBakeId, TriangleIdSource};

const FLOOR_X: f32 = 10.0;

fn scratch_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dynlight-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app(assets: &Path) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin {
            file_path: assets.to_string_lossy().into_owned(),
            ..default()
        },
        TransformPlugin,
    ))
    .init_asset::<Mesh>()
    .add_plugins(DynamicLightingPlugin)
    .insert_resource(BakeSettings {
        pixel_density: 8.0,
        ..default()
    });
    app.finish();
    app.cleanup();
    app
}

/// A 4 m floor at `x = 10` under a level root, a floating 1 m box, one baked light above the box
/// and one realtime light that must not be baked.
fn spawn_scene(app: &mut App) {
    let mut floor = Plane3d::default().mesh().size(4.0, 4.0).build();
    let uv0 = floor.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().clone();
    floor.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv0);
    let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
    let floor = meshes.add(floor);
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0).mesh().build());

    let world = app.world_mut();
    world
        .spawn((Name::new("Level"), Transform::from_xyz(FLOOR_X, 0.0, 0.0)))
        .with_child((Name::new("Floor"), Mesh3d(floor), BakeStatic));
    world.spawn((
        Mesh3d(cube),
        BakeStatic,
        Transform::from_xyz(FLOOR_X, 1.0, 0.0),
    ));
    world.spawn((
        DynamicLight {
            radius: 6.0,
            ..default()
        },
        BakeId(42),
        Transform::from_xyz(FLOOR_X, 3.0, 0.0),
    ));
    world.spawn((
        DynamicLight {
            realtime: true,
            ..default()
        },
        Transform::from_xyz(0.0, 3.0, 0.0),
    ));
}

/// Updates until `count` requests have finished.
fn run_until_finished(app: &mut App, count: usize) -> Vec<BakeFinished> {
    let mut finished = Vec::new();
    for _ in 0..6000 {
        app.update();
        let mut messages = app.world_mut().resource_mut::<Messages<BakeFinished>>();
        finished.extend(messages.drain());
        if finished.len() >= count {
            return finished;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("only {} of {count} bakes finished", finished.len());
}

/// Whether the floor texel `(x, y)` (32² map) is lit, read like the shader does. The plane's
/// two triangles split along `u + v = 1`.
fn floor_lit(scene: &BakedScene, x: u32, y: u32) -> bool {
    let floor = &scene.meshes[scene
        .meshes
        .iter()
        .position(|m| m.id == BakeId::from_path(["Level", "Floor"]))
        .expect("derived floor id")];
    assert_eq!(floor.resolution, 32, "ceil(sqrt(8² · 16 m²))");
    let triangle = if x + y + 1 > floor.resolution { 0 } else { 1 };
    let view = TriangleView::load(&floor.triangles, triangle, scene.settings.bounce);
    assert_eq!(view.light_count, 1);
    let entry = view.light(0);
    assert_eq!(entry.light_index, 0);
    view.shadow_bit(&entry, x, y).expect("shadow data")
}

#[test]
fn bake_request_writes_a_loadable_file() {
    let dir = scratch_dir("bake-ecs");
    let mut app = app(&dir);
    spawn_scene(&mut app);
    app.update();

    let output = dir.join("scene.dynlight");
    app.world_mut().write_message(BakeRequest {
        output: output.clone(),
        preset: BakePreset::Preview,
    });
    let finished = run_until_finished(&mut app, 1).remove(0);
    assert_eq!(finished.output, output);
    let report = finished.result.expect("bake succeeds");
    assert_eq!(report.stats.meshes, 2);
    assert!(report.stats.texels > 0 && report.stats.rays > 0);
    assert!(!app.world().resource::<BakeStatus>().is_running());

    let bytes = std::fs::read(&output).unwrap();
    assert_eq!(report.bytes, bytes.len() as u64);
    let scene = decode(&bytes).unwrap();
    assert_eq!(scene.lights.len(), 1, "realtime lights aren't baked");
    assert_eq!(scene.lights[0].id, BakeId(42));
    assert_eq!(scene.lights[0].origin, Vec3::new(FLOOR_X, 3.0, 0.0));
    assert_eq!(scene.light_bvh.len(), 1);

    // The box shadows the middle of the floor; the corners are lit. Seeing this at x = 10
    // proves the meshes were moved to world space.
    for (x, y) in [(15, 15), (16, 16), (15, 16)] {
        assert!(
            !floor_lit(&scene, x, y),
            "texel ({x}, {y}) is under the box"
        );
    }
    for (x, y) in [(2, 2), (29, 29), (2, 29), (29, 2)] {
        assert!(floor_lit(&scene, x, y), "texel ({x}, {y}) sees the light");
    }

    let handle: Handle<DynLightBake> = app.world().resource::<AssetServer>().load("scene.dynlight");
    for _ in 0..2000 {
        app.update();
        if app
            .world()
            .resource::<Assets<DynLightBake>>()
            .contains(&handle)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let loaded = app.world().resource::<Assets<DynLightBake>>().get(&handle);
    assert_eq!(loaded.expect("asset loads").scene, scene);

    app.insert_resource(ActiveBake(Some(handle)));
    app.update();
    assert_bake_applied(&mut app, &scene);

    // The triangle-ID fallback de-indexes the baked meshes; they must still match the bake.
    app.world_mut()
        .resource_mut::<DynamicLightingSettings>()
        .triangle_ids = TriangleIdSource::VertexAttribute;
    for _ in 0..3 {
        app.update();
    }
    let world = app.world_mut();
    let mut baked = world.query_filtered::<&Mesh3d, With<BakeStatic>>();
    let meshes = world.resource::<Assets<Mesh>>();
    for mesh in baked.iter(world) {
        let mesh = meshes.get(mesh.id()).unwrap();
        assert!(has_triangle_ids(mesh) && mesh.indices().is_none());
    }
    assert_bake_applied(&mut app, &scene);
    std::fs::remove_dir_all(&dir).ok();
}

/// Every baked mesh found its receiver record and the baked light sits in its bake slot.
fn assert_bake_applied(app: &mut App, scene: &BakedScene) {
    let world = app.world_mut();
    let slots = &world.resource::<BakedLightSlots>().slots;
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].id, BakeId(42));
    let receivers = &world.resource::<AppliedBake>().buffers.receivers;
    assert_eq!(receivers.len(), scene.meshes.len());
    let mut expected: Vec<_> = scene
        .meshes
        .iter()
        .map(|mesh| (mesh.id, receivers[&mesh.id].tag))
        .collect();

    let mut query = world.query_filtered::<(&ResolvedBakeId, &MeshTag), With<BakeStatic>>();
    let mut tagged: Vec<_> = query
        .iter(world)
        .map(|(id, tag)| (id.0, tag.value))
        .collect();
    tagged.sort();
    expected.sort();
    assert_eq!(tagged, expected, "runtime ids match the baked ids");

    let mut lights = world.query::<(&DynamicLight, Option<&ResolvedBakeId>)>();
    let ids: Vec<_> = lights
        .iter(world)
        .map(|(l, id)| (l.realtime, id.copied()))
        .collect();
    assert!(ids.contains(&(false, Some(ResolvedBakeId(BakeId(42))))));
    assert!(
        ids.contains(&(true, None)),
        "realtime lights have no bake id"
    );
    assert_eq!(
        world.resource::<PackedLights>().lights.len(),
        1,
        "one baked slot"
    );
}

#[test]
fn render_world_only_meshes_are_reported() {
    let dir = scratch_dir("bake-ecs-extracted");
    let mut app = app(&dir);
    let mut mesh = Cuboid::new(1.0, 1.0, 1.0).mesh().build();
    mesh.take_gpu_data().unwrap();
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    app.world_mut()
        .spawn((Name::new("Crate"), Mesh3d(mesh), BakeStatic));
    app.update();

    let output = dir.join("never.dynlight");
    app.world_mut().write_message(BakeRequest {
        output: output.clone(),
        preset: BakePreset::Full,
    });
    let finished = run_until_finished(&mut app, 1).remove(0);
    let Err(BakeFailure::Snapshot(SnapshotError::MeshData { entity, reason })) = finished.result
    else {
        panic!("expected a mesh data error, got {:?}", finished.result);
    };
    assert!(entity.contains("Crate"), "{entity}");
    assert!(reason.contains("RenderAssetUsages::MAIN_WORLD"), "{reason}");
    assert!(!output.exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn second_request_while_running_is_busy() {
    let dir = scratch_dir("bake-ecs-busy");
    let mut app = app(&dir);
    spawn_scene(&mut app);
    app.update();
    for name in ["a.dynlight", "b.dynlight"] {
        app.world_mut().write_message(BakeRequest {
            output: dir.join(name),
            preset: BakePreset::Preview,
        });
    }
    let finished = run_until_finished(&mut app, 2);
    assert_eq!(finished[0].output, dir.join("b.dynlight"));
    assert_eq!(finished[0].result, Err(BakeFailure::Busy));
    assert_eq!(finished[1].output, dir.join("a.dynlight"));
    assert!(finished[1].result.is_ok());
    std::fs::remove_dir_all(&dir).ok();
}
