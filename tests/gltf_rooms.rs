//! glTF rooms and portals through Bevy's own loader: `tests/fixtures/rooms.gltf`, exported by
//! Blender from `tests/fixtures/rooms_gltf.py`, whose docs describe the scene.
//!
//! In Bevy coordinates Room A spans (8, 0, -3) to (12, 3, 3) and Room B (8, 0, -7) to
//! (12, 3, -3); the door joins them at z = -3 and the closed window opens Room B to the outside
//! at z = -7.

use std::path::Path;
use std::time::{Duration, Instant};

use bevy::asset::LoadState;
use bevy::asset::io::memory::{Dir, MemoryAssetReader};
use bevy::asset::io::{AssetSourceBuilder, AssetSourceId};
use bevy::gltf::{GltfAssetLabel, GltfPlugin};
use bevy::mesh::MeshPlugin;
use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use bevy::world_serialization::{WorldAssetRoot, WorldSerializationPlugin};
use dynlight::portals::{OUTSIDE, PortalState, PortalStats, RoomGraph};
use dynlight::prelude::*;

fn app() -> App {
    let dir = Dir::default();
    let files: [(&str, &'static [u8]); 2] = [
        ("rooms.gltf", include_bytes!("fixtures/rooms.gltf")),
        ("rooms.bin", include_bytes!("fixtures/rooms.bin")),
    ];
    for (path, bytes) in files {
        dir.insert_asset(Path::new(path), bytes);
    }
    let reader = MemoryAssetReader { root: dir };
    let mut app = App::new();
    app.register_asset_source(
        AssetSourceId::Default,
        AssetSourceBuilder::new(move || Box::new(reader.clone())),
    )
    .add_plugins((
        MinimalPlugins,
        TransformPlugin,
        AssetPlugin::default(),
        WorldSerializationPlugin,
        MeshPlugin,
        GltfPlugin::default(),
        DynamicLightingPlugin,
    ));
    app.finish();
    app.cleanup();
    app
}

fn named(world: &mut World, name: &str) -> Option<Entity> {
    let mut nodes = world.query::<(Entity, &Name)>();
    nodes
        .iter(world)
        .find(|(_, node)| node.as_str() == name)
        .map(|(entity, _)| entity)
}

/// Loads the scene and runs frames until it has spawned, stopping on that first frame.
fn spawned_scene(app: &mut App) {
    let scene = app
        .world()
        .resource::<AssetServer>()
        .load(GltfAssetLabel::Scene(0).from_asset("rooms.gltf"));
    let id = scene.id();
    app.world_mut().spawn(WorldAssetRoot(scene));
    let start = Instant::now();
    while named(app.world_mut(), "Level").is_none() {
        let server = app.world().resource::<AssetServer>();
        if let Some(LoadState::Failed(error)) = server.get_load_state(id) {
            panic!("rooms.gltf failed to load: {error}");
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "rooms.gltf never spawned"
        );
        app.update();
    }
}

/// The world bounds of `points` in `entity`'s local space.
fn world_box(world: &World, entity: Entity, points: &[Vec3]) -> (Vec3, Vec3) {
    let transform = world.get::<GlobalTransform>(entity).expect("a transform");
    points
        .iter()
        .map(|&point| transform.transform_point(point))
        .fold((Vec3::MAX, Vec3::MIN), |(min, max), point| {
            (min.min(point), max.max(point))
        })
}

fn assert_box(found: (Vec3, Vec3), min: Vec3, max: Vec3, what: &str) {
    let close = found.0.abs_diff_eq(min, 1e-4) && found.1.abs_diff_eq(max, 1e-4);
    assert!(close, "{what}: {found:?}, expected {min} to {max}");
}

fn cube() -> Vec<Vec3> {
    (0..8)
        .map(|corner| {
            let sign = |bit: u32| if corner >> bit & 1 == 0 { -1.0 } else { 1.0 };
            Vec3::new(sign(0), sign(1), sign(2))
        })
        .collect()
}

fn square() -> Vec<Vec3> {
    cube()
        .into_iter()
        .map(|corner| corner.with_z(0.0))
        .collect()
}

/// The `Mesh3d` children of `node`.
fn primitives(world: &World, node: Entity) -> Vec<Entity> {
    world.get::<Children>(node).map_or(Vec::new(), |children| {
        children
            .iter()
            .filter(|&child| world.entity(child).contains::<Mesh3d>())
            .collect()
    })
}

#[test]
fn rooms_and_portals_from_blender_are_in_place_on_the_first_frame() {
    let mut app = app();
    spawned_scene(&mut app);
    let world = app.world_mut();
    let [level, room_a, room_b, door, window, lamp, floor, crate_box] = [
        "Level", "Room A", "Room B", "Door", "Window", "Lamp", "Floor", "Crate",
    ]
    .map(|name| named(world, name).unwrap_or_else(|| panic!("no node {name}")));
    let world = app.world();
    let room = |entity: Entity| world.get::<Room>(entity).map(|room| room.half_size);
    assert_eq!(
        (room(room_a), room(room_b)),
        (Some(Vec3::ONE), Some(Vec3::ONE))
    );
    let opening = Some(&Portal::new(Vec2::splat(2.0)));
    assert_eq!((world.get(door), world.get(window)), (opening, opening));
    assert!(!world.entity(door).contains::<ClosedPortal>());
    assert!(world.entity(window).contains::<ClosedPortal>());

    assert_box(
        world_box(world, room_a, &cube()),
        vec3(8.0, 0.0, -3.0),
        vec3(12.0, 3.0, 3.0),
        "Room A",
    );
    assert_box(
        world_box(world, room_b, &cube()),
        vec3(8.0, 0.0, -7.0),
        vec3(12.0, 3.0, -3.0),
        "Room B",
    );
    assert_box(
        world_box(world, door, &square()),
        vec3(9.4, 0.0, -3.0),
        vec3(10.6, 2.2, -3.0),
        "Door",
    );
    assert_box(
        world_box(world, window, &square()),
        vec3(9.0, 0.9, -7.0),
        vec3(11.0, 2.1, -7.0),
        "Window",
    );

    assert!(
        primitives(world, room_b).is_empty(),
        "Room B's own meshes stay"
    );
    assert_eq!(
        world.get::<ChildOf>(crate_box).map(ChildOf::parent),
        Some(room_b)
    );
    assert_eq!(
        primitives(world, crate_box).len(),
        1,
        "the crate lost its mesh"
    );
    let floor_meshes = primitives(world, floor);
    assert!(!floor_meshes.is_empty());
    for mesh in floor_meshes {
        assert!(world.entity(mesh).contains::<BakeStatic>());
    }
    assert!(world.get::<ChildOf>(level).is_some());

    let world = app.world_mut();
    let mut lights = world.query::<(Entity, &DynamicLight)>();
    let lights: Vec<_> = lights.iter(world).collect();
    assert_eq!(lights.len(), 1, "only the lamp is a light");
    assert_eq!(lights[0].0, lamp);
    assert_eq!(
        (lights[0].1.light_type, lights[0].1.radius),
        (LightType::Point, 3.0)
    );
    let helpers = world
        .query_filtered::<Entity, Or<(With<Room>, With<Portal>)>>()
        .iter(world)
        .count();
    assert_eq!(helpers, 4);
}

#[test]
fn the_imported_rooms_link_through_the_door_and_the_window() {
    let mut app = app();
    spawned_scene(&mut app);
    app.update();
    let world = app.world_mut();
    let [room_a, room_b, door, window] =
        ["Room A", "Room B", "Door", "Window"].map(|name| named(world, name).expect(name));
    let rooms = world.resource::<RoomGraph>();
    assert_eq!(*rooms.state(), PortalState::Ready);
    let graph = rooms.ready().expect("a ready graph");
    let node = |room: Entity| 1 + rooms.rooms.iter().position(|&r| r == room).expect("a cell");
    let sides = |portal: Entity| {
        let index = rooms
            .portals
            .iter()
            .position(|&p| p == portal)
            .expect("linked");
        let mut sides = graph.portals[index].sides.clone().to_vec();
        sides.sort();
        sides
    };
    assert_eq!(sides(door), {
        let mut expected = vec![vec![node(room_a)], vec![node(room_b)]];
        expected.sort();
        expected
    });
    assert_eq!(sides(window), {
        let mut expected = vec![vec![node(room_b)], vec![OUTSIDE]];
        expected.sort();
        expected
    });
    let stats = world.resource::<PortalStats>();
    assert_eq!(
        (stats.rooms, stats.portals, stats.closed_portals),
        (2, 2, 1)
    );
}
