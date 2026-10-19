//! Node extras in an app: when they are read, which entities they tag, and what a helper keeps.

use bevy::gltf::GltfExtras;
use bevy::prelude::*;

use super::GltfImportPlugin;
use crate::bake::BakeStatic;
use crate::light::DynamicLight;
use crate::portals::{ClosedPortal, Portal, Room};

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, GltfImportPlugin));
    app
}

fn extras(json: &str) -> GltfExtras {
    GltfExtras {
        value: json.to_owned(),
    }
}

#[test]
fn a_static_node_tags_its_own_primitives_and_not_its_child_nodes() {
    let mut app = app();
    let world = app.world_mut();
    let node = world.spawn(extras(r#"{"dynlight_static": true}"#)).id();
    let primitive = world.spawn((Mesh3d::default(), ChildOf(node))).id();
    let child_node = world.spawn(ChildOf(node)).id();
    let child_primitive = world.spawn((Mesh3d::default(), ChildOf(child_node))).id();
    app.update();
    let tagged = |entity: Entity| app.world().entity(entity).contains::<BakeStatic>();
    assert!(tagged(node) && tagged(primitive));
    assert!(!tagged(child_node) && !tagged(child_primitive));
}

/// Whether the light component was on the node each time `PostUpdate` ran.
#[derive(Resource, Default)]
struct Seen(Vec<bool>);

#[test]
fn a_node_spawned_in_update_is_a_light_before_post_update() {
    let mut app = app();
    app.init_resource::<Seen>()
        .add_systems(
            Update,
            |mut commands: Commands, mut spawned: Local<bool>| {
                if !std::mem::replace(&mut *spawned, true) {
                    commands.spawn(extras(r#"{"dynlight_radius": 4}"#));
                }
            },
        )
        .add_systems(
            PostUpdate,
            |nodes: Query<Has<DynamicLight>, With<GltfExtras>>, mut seen: ResMut<Seen>| {
                seen.0.extend(nodes.iter());
            },
        );
    app.update();
    assert_eq!(app.world().resource::<Seen>().0, [true]);
}

#[test]
fn a_helper_loses_its_own_primitives_and_keeps_its_child_nodes() {
    let mut app = app();
    let world = app.world_mut();
    let door = world
        .spawn(extras(
            r#"{"dynlight_portal": true, "dynlight_portal_closed": 1, "dynlight_radius": 2}"#,
        ))
        .id();
    let door_primitive = world.spawn((Mesh3d::default(), ChildOf(door))).id();
    let room = world.spawn(extras(r#"{"dynlight_room": true}"#)).id();
    let room_primitive = world.spawn((Mesh3d::default(), ChildOf(room))).id();
    let child_node = world.spawn(ChildOf(room)).id();
    let child_primitive = world.spawn((Mesh3d::default(), ChildOf(child_node))).id();
    app.update();
    let world = app.world();
    let door = world.entity(door);
    assert_eq!(door.get::<Portal>(), Some(&Portal::new(Vec2::splat(2.0))));
    assert!(door.contains::<ClosedPortal>() && !door.contains::<DynamicLight>());
    let room = world.entity(room);
    assert_eq!(
        room.get::<Room>().map(|room| room.half_size),
        Some(Vec3::ONE)
    );
    assert!(world.get_entity(door_primitive).is_err());
    assert!(world.get_entity(room_primitive).is_err());
    assert!(world.get_entity(child_node).is_ok() && world.get_entity(child_primitive).is_ok());
}

#[test]
fn a_room_that_is_also_a_portal_keeps_its_mesh_and_makes_nothing() {
    let mut app = app();
    let world = app.world_mut();
    let json = r#"{"dynlight_room": true, "dynlight_portal": true}"#;
    let node = world.spawn(extras(json)).id();
    let primitive = world.spawn((Mesh3d::default(), ChildOf(node))).id();
    app.update();
    let node = app.world().entity(node);
    assert!(!node.contains::<Room>() && !node.contains::<Portal>());
    assert!(app.world().get_entity(primitive).is_ok());
}
