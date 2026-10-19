//! Node extras in an app: when they are read, and which entities they tag.

use bevy::gltf::GltfExtras;
use bevy::prelude::*;

use super::GltfImportPlugin;
use crate::bake::BakeStatic;
use crate::light::DynamicLight;

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
