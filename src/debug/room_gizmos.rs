//! Wireframe gizmos for rooms and portals.

use bevy::prelude::*;

use crate::portals::{
    Aperture, ClosedPortal, OUTSIDE, Parallelepiped, Portal, Room, RoomGraph, RoomView,
};

/// A room some camera sees into (or every room, when no camera's view is bounded).
const SEEN: Color = Color::srgb(0.3, 0.9, 0.4);
/// A room no camera sees into, a portal that joins nothing, or one of a graph that isn't valid.
const UNSEEN: Color = Color::srgb(0.4, 0.4, 0.4);
const BETWEEN_ROOMS: Color = Color::srgb(1.0, 0.85, 0.2);
const TO_OUTSIDE: Color = Color::srgb(1.0, 0.5, 0.1);
/// The rooms and portals that turned portal culling off.
const BROKEN: Color = Color::srgb(1.0, 0.15, 0.1);
/// How much darker a [`ClosedPortal`] is drawn.
const CLOSED_DARKER: f32 = 0.35;

/// Draws every room cell and portal: cells by whether a camera sees into them, portals by what
/// they join (darker when closed), each with a short arrow out of its front.
pub(super) fn draw_room_gizmos(
    mut gizmos: Gizmos,
    graph: Res<RoomGraph>,
    views: Query<(&Camera, &RoomView)>,
    rooms: Query<(Entity, &Room, &GlobalTransform)>,
    portals: Query<(Entity, &Portal, &GlobalTransform)>,
    closed: Query<(), With<ClosedPortal>>,
) {
    let Some(linked) = graph.ready() else {
        let color = |entity| {
            if graph.culprits().contains(&entity) {
                BROKEN
            } else {
                UNSEEN
            }
        };
        for (entity, room, transform) in &rooms {
            if let Some(shape) = Parallelepiped::from_affine(transform.affine(), room.half_size) {
                draw_cell(&mut gizmos, &shape, color(entity));
            }
        }
        for (entity, portal, transform) in &portals {
            if let Some(aperture) = Aperture::from_affine(transform.affine(), portal.size) {
                draw_portal(&mut gizmos, &aperture, color(entity));
            }
        }
        return;
    };
    let traced: Vec<_> = views
        .iter()
        .filter(|(camera, _)| camera.is_active)
        .filter_map(|(_, view)| view.bounded(graph.generation()))
        .collect();
    for (index, cell) in linked.cells.iter().enumerate() {
        let seen = traced.is_empty() || traced.iter().any(|view| view.visits(index + 1));
        draw_cell(&mut gizmos, &cell.shape, if seen { SEEN } else { UNSEEN });
    }
    for (portal, &entity) in linked.portals.iter().zip(&graph.portals) {
        let color = if portal.is_ignored() {
            UNSEEN
        } else if portal.sides.iter().flatten().any(|&node| node == OUTSIDE) {
            TO_OUTSIDE
        } else {
            BETWEEN_ROOMS
        };
        let color = if closed.contains(entity) {
            color.darker(CLOSED_DARKER)
        } else {
            color
        };
        draw_portal(&mut gizmos, &portal.aperture, color);
    }
}

/// Cells are drawn this far inside their faces, which usually lie in the middle of walls.
const INSET: f32 = 0.15;

fn draw_cell(gizmos: &mut Gizmos, shape: &Parallelepiped, color: Color) {
    let mut shape = *shape;
    for axis in &mut shape.axes {
        let length = axis.length();
        *axis *= (length - INSET).max(length * 0.5) / length;
    }
    let corners = shape.corners();
    for (index, &corner) in corners.iter().enumerate() {
        for bit in [1, 2, 4] {
            if index & bit == 0 {
                gizmos.line(corner, corners[index | bit], color);
            }
        }
    }
}

fn draw_portal(gizmos: &mut Gizmos, aperture: &Aperture, color: Color) {
    let corners = aperture.corners();
    gizmos.linestrip(corners.into_iter().chain([corners[0]]), color);
    let size = aperture.u.length().min(aperture.v.length());
    gizmos.arrow(
        aperture.center,
        aperture.center + aperture.normal * size.clamp(0.1, 0.5),
        color,
    );
}
