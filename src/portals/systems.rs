//! The portal systems: rebuild the [`RoomGraph`] when rooms or portals change, then trace every
//! camera's [`RoomView`] through it.

use bevy::camera::CameraProjection;
use bevy::camera::primitives::Frustum;
use bevy::ecs::entity::EntityHashMap;
use bevy::ecs::name::NameOrEntity;
use bevy::prelude::*;

use super::authoring::{ClosedPortal, Portal, PortalSettings, PortalSide, Room};
use super::graph::{Graph, PortalInput, Problem, RoomInput, SideInput};
use super::link::{NEAR_MISS, Unresolved};
use super::near::near_corners;
use super::state::{PortalState, PortalStats, RoomGraph, RoomView};
use super::trace::{TraceCounts, Viewer, trace};

type ChangedRooms = (With<Room>, Or<(Changed<Room>, Changed<GlobalTransform>)>);
type ChangedPortals = (
    With<Portal>,
    Or<(Changed<Portal>, Changed<GlobalTransform>)>,
);

/// Rebuilds the graph when a room, a portal, their transforms or the settings change, or one
/// is removed; logs why culling is off when the rooms break the rules.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_graph(
    settings: Res<PortalSettings>,
    rooms: Query<(Entity, &Room, &GlobalTransform)>,
    portals: Query<(Entity, &Portal, &GlobalTransform)>,
    (changed_rooms, changed_portals): (Query<(), ChangedRooms>, Query<(), ChangedPortals>),
    (mut removed_rooms, mut removed_portals): (RemovedComponents<Room>, RemovedComponents<Portal>),
    names: Query<NameOrEntity>,
    mut graph: ResMut<RoomGraph>,
    mut warned: Local<(String, usize)>,
) {
    let removed = removed_rooms.read().count() + removed_portals.read().count() > 0;
    if !(settings.is_changed()
        || removed
        || !changed_rooms.is_empty()
        || !changed_portals.is_empty())
    {
        return;
    }
    let graph = &mut *graph;
    graph.generation += 1;
    graph.rooms = rooms.iter().map(|(entity, ..)| entity).collect();
    graph.portals = portals.iter().map(|(entity, ..)| entity).collect();
    graph.graph = Graph::default();
    graph.culprits.clear();
    if !settings.enabled || graph.rooms.is_empty() {
        graph.state = PortalState::Empty;
        return;
    }

    let index: EntityHashMap<usize> = graph
        .rooms
        .iter()
        .enumerate()
        .map(|(index, &room)| (room, index))
        .collect();
    let side = |side: &PortalSide| match side {
        PortalSide::Auto => SideInput::Auto,
        PortalSide::Into { rooms, outside } => SideInput::Into {
            // An entity that isn't a room fails the build.
            rooms: rooms
                .iter()
                .map(|room| index.get(room).copied().unwrap_or(usize::MAX))
                .collect(),
            outside: *outside,
        },
    };
    let room_inputs: Vec<RoomInput> = rooms
        .iter()
        .map(|(_, room, transform)| RoomInput {
            affine: transform.affine(),
            half: room.half_size,
        })
        .collect();
    let portal_inputs: Vec<PortalInput> = portals
        .iter()
        .map(|(_, portal, transform)| PortalInput {
            affine: transform.affine(),
            size: portal.size,
            sides: [side(&portal.back), side(&portal.front)],
        })
        .collect();

    let tolerance = settings.tolerance.max(0.0);
    match Graph::build(&room_inputs, &portal_inputs, tolerance) {
        Ok(built) => {
            graph.graph = built;
            graph.state = PortalState::Ready;
            warned.0.clear();
            let ignored: Vec<String> = graph
                .graph
                .portals
                .iter()
                .zip(&graph.portals)
                .filter(|(portal, _)| portal.is_ignored())
                .map(|(_, &entity)| name(&names, entity))
                .collect();
            if ignored.len() != warned.1 && !ignored.is_empty() {
                warn!(
                    "dynlight portals: {} portal(s) open into the same room or the outside on both sides and join nothing: {}",
                    ignored.len(),
                    ignored.join(", ")
                );
            }
            warned.1 = ignored.len();
        }
        Err(problem) => {
            let reason = describe(&problem, graph, &names, tolerance);
            if warned.0 != reason {
                warn!("dynlight portals: {reason}; portal culling is off until that is fixed");
                warned.0 = reason.clone();
            }
            graph.state = PortalState::Disabled(reason);
            graph.culprits = culprits(&problem, graph);
        }
    }
}

fn culprits(problem: &Problem, graph: &RoomGraph) -> Vec<Entity> {
    match problem {
        Problem::Room(index) => vec![graph.rooms[*index]],
        Problem::Portal(index) | Problem::Into { portal: index, .. } => {
            vec![graph.portals[*index]]
        }
        Problem::Overlap(a, b, _) => vec![graph.rooms[*a], graph.rooms[*b]],
        Problem::Side {
            portal,
            issue: Unresolved::Uncovered(cells) | Unresolved::Several(cells),
            ..
        } => core::iter::once(graph.portals[*portal])
            .chain(cells.iter().map(|&cell| graph.rooms[cell]))
            .collect(),
    }
}

fn name(names: &Query<NameOrEntity>, entity: Entity) -> String {
    names
        .get(entity)
        .map_or_else(|_| format!("{entity}"), |name| format!("\"{name}\""))
}

/// The problem in words, with the rooms and portals by name.
fn describe(
    problem: &Problem,
    graph: &RoomGraph,
    names: &Query<NameOrEntity>,
    tolerance: f32,
) -> String {
    let room = |index: usize| name(names, graph.rooms[index]);
    let rooms = |indices: &[usize]| indices.iter().map(|&i| room(i)).collect::<Vec<_>>();
    let portal = |index: usize| name(names, graph.portals[index]);
    let side = |side: usize| if side == 0 { "back" } else { "front" };
    match problem {
        Problem::Room(index) => format!("room {} is flat, singular or not finite", room(*index)),
        Problem::Portal(index) => format!("portal {} is flat or not finite", portal(*index)),
        Problem::Overlap(a, b, depth) => format!(
            "rooms {} and {} overlap by {depth:.3} m, more than the {tolerance} m tolerance",
            room(*a),
            room(*b)
        ),
        Problem::Side {
            portal: index,
            side: which,
            issue: Unresolved::Uncovered(cells),
        } => format!(
            "the {} of portal {} has room(s) {} within {NEAR_MISS} m, but none takes the whole \
             opening within {tolerance} m of it; extend the room to the portal, or set \
             PortalSide::Into",
            side(*which),
            portal(*index),
            rooms(cells).join(", ")
        ),
        Problem::Side {
            portal: index,
            side: which,
            issue: Unresolved::Several(cells),
        } => format!(
            "the {} of portal {} opens into several rooms at once ({})",
            side(*which),
            portal(*index),
            rooms(cells).join(", ")
        ),
        Problem::Into {
            portal: index,
            side: which,
        } => format!(
            "the {} of portal {} is set to open into nothing, or into an entity that isn't a Room",
            side(*which),
            portal(*index)
        ),
    }
}

type CameraItem<'a> = (
    Entity,
    &'a Camera,
    &'a GlobalTransform,
    &'a Frustum,
    &'a Projection,
    Option<&'a mut RoomView>,
);

/// Traces every active perspective camera through the graph into its [`RoomView`] (added the
/// first time), past the portals that aren't [`ClosedPortal`]; every other camera gets an
/// unbounded view.
pub(super) fn trace_views(
    mut commands: Commands,
    graph: Res<RoomGraph>,
    settings: Res<PortalSettings>,
    shut: Query<(), With<ClosedPortal>>,
    mut cameras: Query<CameraItem>,
    mut stats: ResMut<PortalStats>,
    mut closed: Local<Vec<bool>>,
) {
    closed.clear();
    closed.extend(graph.portals.iter().map(|&portal| shut.contains(portal)));
    let mut counts = TraceCounts::default();
    let (mut views, mut unbounded) = (0, 0);
    for (entity, camera, transform, frustum, projection, view) in &mut cameras {
        let mut added = RoomView::default();
        let is_new = view.is_none();
        let view = match view {
            Some(view) => view.into_inner(),
            None => &mut added,
        };
        view.generation = graph.generation();
        view.bounded = match (graph.ready(), camera.is_active, projection) {
            (Some(rooms), true, Projection::Perspective(perspective)) => {
                views += 1;
                let planes = frustum.half_spaces.map(|half_space| half_space.normal_d());
                let eye = transform.translation();
                let clip_from_view = match &camera.sub_camera_view {
                    Some(sub) => perspective.get_clip_from_view_for_sub(sub),
                    None => perspective.get_clip_from_view(),
                };
                let near = near_corners(clip_from_view, transform.affine());
                let viewer = Viewer {
                    eye,
                    near: near.as_ref().map_or(&[], |near| near.as_slice()),
                    frustum: &planes,
                };
                let bounded = trace(
                    rooms,
                    viewer,
                    &closed,
                    settings.depth_limit,
                    &mut view.traced,
                    &mut counts,
                );
                unbounded += u32::from(!bounded);
                bounded
            }
            _ => false,
        };
        if is_new {
            commands.entity(entity).insert(added);
        }
    }
    *stats = PortalStats {
        state: graph.state().clone(),
        rooms: graph.rooms.len() as u32,
        portals: graph.portals.len() as u32,
        closed_portals: closed.iter().filter(|&&closed| closed).count() as u32,
        ignored_portals: graph.ready().map_or(0, |rooms| {
            rooms
                .portals
                .iter()
                .filter(|portal| portal.is_ignored())
                .count() as u32
        }),
        views,
        unbounded_views: unbounded,
        visits: counts.visits,
        portal_tests: counts.portal_tests,
        plane_tests: counts.plane_tests,
    };
}
