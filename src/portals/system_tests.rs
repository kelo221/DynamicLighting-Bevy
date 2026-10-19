//! The portal systems in an app: when the graph rebuilds, which cameras get bounded views, and
//! that the light-picking slices see those views in the frame they are traced.

use bevy::camera::primitives::Frustum;
use bevy::prelude::*;
use bevy::transform::TransformPlugin;

use super::fixtures;
use super::{
    ClosedPortal, Portal, PortalSettings, PortalSide, PortalState, PortalStats, PortalSystems,
    PortalsPlugin, Room, RoomGraph, RoomView, sees,
};

const EYE: Vec3 = Vec3::new(3.0, 1.5, 0.0);
const TARGET: Vec3 = Vec3::new(9.0, 1.5, 0.0);
/// In the second room, in the frustum, but behind the wall beside the door.
const HIDDEN: Vec3 = Vec3::new(9.0, 1.5, 2.0);
/// In the second room, through the door.
const THROUGH: Vec3 = Vec3::new(9.0, 1.5, 0.0);
const RADIUS: f32 = 0.5;

/// What a system ordered after [`PortalSystems::Trace`] sees of [`HIDDEN`], frame by frame.
#[derive(Resource, Default)]
struct Probe(Vec<bool>);

fn probe(
    graph: Res<RoomGraph>,
    cameras: Query<(&Frustum, Option<&RoomView>)>,
    mut seen: ResMut<Probe>,
) {
    seen.0.push(sees(&graph, cameras.iter(), HIDDEN, RADIUS));
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, PortalsPlugin))
        .init_resource::<Probe>()
        .add_systems(PostUpdate, probe.after(PortalSystems::Trace));
    app
}

/// Two 6 m rooms along +X joined by a door at `x = 6`: `[first, second, door]`.
fn rooms(world: &mut World) -> [Entity; 3] {
    let size = Vec3::new(6.0, 3.0, 6.0);
    let first = world
        .spawn((Room::new(size), Transform::from_xyz(3.0, 1.5, 0.0)))
        .id();
    let second = world
        .spawn((Room::new(size), Transform::from_xyz(9.0, 1.5, 0.0)))
        .id();
    let door = world
        .spawn((
            Portal::new(Vec2::new(1.0, 2.0)),
            Transform::from_xyz(6.0, 1.0, 0.0).looking_to(Vec3::NEG_X, Vec3::Y),
        ))
        .id();
    [first, second, door]
}

fn camera(world: &mut World, eye: Vec3, target: Vec3) -> Entity {
    world
        .spawn((
            Camera::default(),
            Projection::Perspective(PerspectiveProjection::default()),
            Transform::from_translation(eye).looking_at(target, Vec3::Y),
            fixtures::camera(eye, target),
        ))
        .id()
}

/// Whether the cameras may see the sphere at `center` right now.
fn sees_now(app: &mut App, center: Vec3) -> bool {
    let world = app.world_mut();
    let mut cameras = world.query::<(&Frustum, Option<&RoomView>)>();
    let world = &*world;
    sees(world.resource(), cameras.iter(world), center, RADIUS)
}

fn generation(app: &App) -> u64 {
    app.world().resource::<RoomGraph>().generation()
}

fn stats(app: &App) -> &PortalStats {
    app.world().resource::<PortalStats>()
}

fn last_probe(app: &App) -> bool {
    *app.world().resource::<Probe>().0.last().unwrap()
}

#[test]
fn views_cull_from_the_first_frame() {
    let mut app = app();
    rooms(app.world_mut());
    camera(app.world_mut(), EYE, TARGET);
    app.update();
    assert_eq!(app.world().resource::<Probe>().0, [false]);
    assert!(sees_now(&mut app, THROUGH));
    let stats = stats(&app);
    assert_eq!(stats.state, PortalState::Ready);
    assert_eq!((stats.rooms, stats.portals), (2, 1));
    assert_eq!((stats.views, stats.unbounded_views), (1, 0));
    assert_eq!(stats.visits, 2);
}

#[test]
fn the_graph_rebuilds_only_when_rooms_or_portals_change() {
    let mut app = app();
    let [first, second, door] = rooms(app.world_mut());
    camera(app.world_mut(), EYE, TARGET);
    app.update();
    let built = generation(&app);
    app.update();
    app.update();
    assert_eq!(generation(&app), built, "nothing changed");
    app.world_mut().entity_mut(first).insert(Name::new("Hall"));
    app.update();
    assert_eq!(generation(&app), built, "a name changes no link");

    app.world_mut()
        .get_mut::<Transform>(second)
        .unwrap()
        .translation
        .x += 0.01;
    app.update();
    assert_eq!(generation(&app), built + 1, "a room moved");
    assert!(!last_probe(&app));

    app.world_mut().get_mut::<Portal>(door).unwrap().size.x = 1.5;
    app.update();
    assert_eq!(generation(&app), built + 2, "a portal changed");

    app.world_mut().despawn(door);
    app.update();
    assert_eq!(generation(&app), built + 3, "a portal removed");
    assert_eq!(stats(&app).portals, 0);
    assert!(!sees_now(&mut app, THROUGH), "the door is gone");

    app.world_mut().despawn(first);
    app.world_mut().despawn(second);
    app.update();
    assert_eq!(stats(&app).state, PortalState::Empty);
    assert_eq!(stats(&app).unbounded_views, 0);
    assert!(last_probe(&app), "no rooms: the frustum alone culls");
}

#[test]
fn moving_or_changing_a_rooms_parent_moves_its_cell() {
    let mut app = app();
    let [_, second, _] = rooms(app.world_mut());
    let wing = app.world_mut().spawn(Transform::IDENTITY).id();
    app.world_mut().entity_mut(second).insert(ChildOf(wing));
    camera(app.world_mut(), EYE, TARGET);
    app.update();
    let center_z = |app: &App| {
        let graph = app.world().resource::<RoomGraph>();
        let index = graph.rooms.iter().position(|&room| room == second).unwrap();
        graph.ready().unwrap().cells[index].shape.center.z
    };
    assert_eq!(center_z(&app), 0.0);
    let built = generation(&app);

    app.world_mut()
        .get_mut::<Transform>(wing)
        .unwrap()
        .translation
        .z = 0.01;
    app.update();
    assert_eq!(generation(&app), built + 1);
    assert!((center_z(&app) - 0.01).abs() < 1e-6);

    let other = app
        .world_mut()
        .spawn(Transform::from_xyz(0.0, 0.0, -0.01))
        .id();
    app.world_mut().entity_mut(second).insert(ChildOf(other));
    app.update();
    assert_eq!(generation(&app), built + 2);
    assert!((center_z(&app) + 0.01).abs() < 1e-6);
    assert!(!last_probe(&app));
}

#[test]
fn orthographic_and_inactive_cameras_cull_nothing() {
    let mut app = app();
    rooms(app.world_mut());
    let camera = camera(app.world_mut(), EYE, TARGET);
    app.update();
    assert!(!last_probe(&app));

    *app.world_mut().get_mut::<Projection>(camera).unwrap() =
        Projection::Orthographic(OrthographicProjection::default_3d());
    app.update();
    assert!(last_probe(&app), "orthographic");
    assert_eq!(stats(&app).views, 0);

    *app.world_mut().get_mut::<Projection>(camera).unwrap() =
        Projection::Perspective(PerspectiveProjection::default());
    app.world_mut().get_mut::<Camera>(camera).unwrap().is_active = false;
    app.update();
    assert!(last_probe(&app), "inactive");

    app.world_mut().get_mut::<Camera>(camera).unwrap().is_active = true;
    app.update();
    assert!(!last_probe(&app), "active again");
}

#[test]
fn turning_portals_off_and_on() {
    let mut app = app();
    rooms(app.world_mut());
    camera(app.world_mut(), EYE, TARGET);
    app.update();
    app.world_mut().resource_mut::<PortalSettings>().enabled = false;
    app.update();
    assert_eq!(stats(&app).state, PortalState::Empty);
    assert!(last_probe(&app));
    app.world_mut().resource_mut::<PortalSettings>().enabled = true;
    app.update();
    assert_eq!(stats(&app).state, PortalState::Ready);
    assert!(!last_probe(&app));
}

#[test]
fn a_view_of_an_older_graph_culls_nothing() {
    let mut app = app();
    rooms(app.world_mut());
    camera(app.world_mut(), EYE, TARGET);
    app.update();
    assert!(!sees_now(&mut app, HIDDEN));
    app.world_mut().resource_mut::<RoomGraph>().generation += 1;
    assert!(sees_now(&mut app, HIDDEN));
    app.update();
    assert!(!sees_now(&mut app, HIDDEN), "traced again");
}

#[test]
fn overlapping_rooms_turn_culling_off_and_name_the_rooms() {
    let mut app = app();
    let [first, ..] = rooms(app.world_mut());
    app.world_mut().entity_mut(first).insert(Name::new("Hall"));
    let booth = app
        .world_mut()
        .spawn((
            Name::new("Booth"),
            Room::new(Vec3::splat(2.0)),
            Transform::from_xyz(4.0, 1.5, 0.0),
        ))
        .id();
    camera(app.world_mut(), EYE, TARGET);
    app.update();
    let PortalState::Disabled(reason) = &stats(&app).state else {
        panic!("{:?}", stats(&app).state);
    };
    assert!(
        reason.contains("\"Hall\"") && reason.contains("\"Booth\""),
        "{reason}"
    );
    let culprits = app.world().resource::<RoomGraph>().culprits();
    assert!(culprits.len() == 2 && culprits.contains(&first) && culprits.contains(&booth));
    assert!(last_probe(&app));

    app.world_mut().despawn(booth);
    app.update();
    assert_eq!(stats(&app).state, PortalState::Ready);
    assert!(!last_probe(&app));
}

#[test]
fn a_side_set_to_an_entity_that_is_not_a_room_turns_culling_off() {
    let mut app = app();
    let [.., door] = rooms(app.world_mut());
    let camera = camera(app.world_mut(), EYE, TARGET);
    app.world_mut().get_mut::<Portal>(door).unwrap().front = PortalSide::Into {
        rooms: vec![camera],
        outside: false,
    };
    app.update();
    assert!(matches!(stats(&app).state, PortalState::Disabled(_)));
    assert!(last_probe(&app));
}

#[test]
fn a_room_sheared_by_its_parent_still_culls() {
    let mut app = app();
    let parent = app
        .world_mut()
        .spawn(Transform::from_scale(Vec3::new(2.0, 1.0, 1.0)))
        .id();
    app.world_mut().spawn((
        Room::new(Vec3::splat(2.0)),
        Transform::from_xyz(0.0, 1.5, 0.0).with_rotation(Quat::from_rotation_y(0.7)),
        ChildOf(parent),
    ));
    let eye = Vec3::new(0.0, 1.5, 0.0);
    camera(app.world_mut(), eye, Vec3::new(10.0, 1.5, 0.0));
    app.update();
    assert_eq!(stats(&app).state, PortalState::Ready);
    assert_eq!(stats(&app).unbounded_views, 0);
    let graph = app.world().resource::<RoomGraph>();
    let axes = graph.ready().unwrap().cells[0].shape.axes;
    assert!(
        axes[0].normalize().dot(axes[2].normalize()).abs() > 0.1,
        "sheared"
    );
    // The room is closed: a sphere outside it, ahead in the frustum, is culled.
    assert!(!sees_now(&mut app, Vec3::new(10.0, 1.5, 0.0)));
    assert!(sees_now(&mut app, Vec3::new(1.5, 1.5, 0.0)));
}

/// The door to shut (or open) from `Update`, through `Commands`.
#[derive(Resource)]
struct Shut(Entity, bool);

fn shut_door(shut: Res<Shut>, mut commands: Commands) {
    if shut.1 {
        commands.entity(shut.0).insert(ClosedPortal);
    } else {
        commands.entity(shut.0).remove::<ClosedPortal>();
    }
}

#[test]
fn a_door_shut_in_update_culls_that_frame_without_a_rebuild() {
    let mut app = app();
    let [_, _, door] = rooms(app.world_mut());
    camera(app.world_mut(), EYE, TARGET);
    app.insert_resource(Shut(door, false))
        .add_systems(Update, shut_door);
    app.update();
    let built = generation(&app);
    assert!(sees_now(&mut app, THROUGH));
    for shut in [true, false] {
        app.world_mut().resource_mut::<Shut>().1 = shut;
        app.update();
        assert_eq!(sees_now(&mut app, THROUGH), !shut);
        assert_eq!(stats(&app).closed_portals, u32::from(shut));
    }
    assert_eq!(generation(&app), built);
}

#[test]
fn a_door_spawned_shut_is_shut_in_the_rebuilt_graph() {
    let mut app = app();
    rooms(app.world_mut());
    let viewer = camera(app.world_mut(), EYE, TARGET);
    app.update();
    let built = generation(&app);
    // A third room past the second, behind a door spawned shut; the camera moves into the
    // second room, looking at the door.
    let world = app.world_mut();
    world.spawn((
        Room::new(Vec3::new(6.0, 3.0, 6.0)),
        Transform::from_xyz(15.0, 1.5, 0.0),
    ));
    let door = world
        .spawn((
            Portal::new(Vec2::new(1.0, 2.0)),
            Transform::from_xyz(12.0, 1.0, 0.0).looking_to(Vec3::NEG_X, Vec3::Y),
            ClosedPortal,
        ))
        .id();
    let (eye, target) = (Vec3::new(8.0, 1.5, 0.0), Vec3::new(15.0, 1.5, 0.0));
    world.entity_mut(viewer).insert((
        Transform::from_translation(eye).looking_at(target, Vec3::Y),
        fixtures::camera(eye, target),
    ));
    app.update();
    let rebuilt = generation(&app);
    assert!(rebuilt > built);
    assert_eq!(stats(&app).closed_portals, 1);
    let past = Vec3::new(16.0, 1.5, 0.5);
    assert!(!sees_now(&mut app, past));
    app.world_mut().entity_mut(door).remove::<ClosedPortal>();
    app.update();
    assert!(sees_now(&mut app, past));
    assert_eq!(generation(&app), rebuilt);
}
