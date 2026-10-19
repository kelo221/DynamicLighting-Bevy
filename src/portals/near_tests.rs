//! Where views start: the eye's nodes plus every node its near pyramid reaches.

use bevy::math::Vec3;

use super::fixtures::{TOLERANCE, camera, near, planes, room, row_graph, sees};
use super::graph::{Graph, OUTSIDE};
use super::near::starts;
use super::trace::{TraceCounts, TracedView, Viewer, trace};

fn starts_of(graph: &Graph, eye: Vec3, target: Vec3) -> Vec<usize> {
    let mut out = Vec::new();
    starts(graph, eye, &near(eye, target), &mut out);
    out.sort_unstable();
    out
}

#[test]
fn the_near_corners_lie_on_the_near_plane() {
    let eye = Vec3::new(1.0, 2.0, 3.0);
    let corners = near(eye, eye + Vec3::NEG_Z);
    let half = 0.1 * (core::f32::consts::FRAC_PI_8).tan();
    for corner in corners {
        let offset = corner - eye;
        assert!((offset.z + 0.1).abs() < 1e-5, "{offset}");
        assert!((offset.x.abs() - half).abs() < 1e-5, "{offset}");
        assert!((offset.y.abs() - half).abs() < 1e-5, "{offset}");
    }
}

#[test]
fn a_pyramid_inside_one_room_starts_there_alone() {
    let graph = row_graph(2, &[0.0]);
    let eye = Vec3::new(3.0, 1.0, 0.0);
    assert_eq!(starts_of(&graph, eye, eye + Vec3::X), [1]);
    // Close to the wall but facing away from it.
    let eye = Vec3::new(5.95, 1.0, -2.0);
    assert_eq!(starts_of(&graph, eye, eye - Vec3::X), [1]);
}

#[test]
fn a_camera_facing_a_thin_wall_starts_behind_it_too() {
    let graph = row_graph(2, &[2.0]);
    // 5 cm from the wall at x = 6, the near plane 0.1 m ahead lies in the next room.
    let eye = Vec3::new(5.95, 1.0, -2.0);
    let target = eye + Vec3::X;
    assert_eq!(starts_of(&graph, eye, target), [1, 2]);

    // A light wholly in the next room, far from the door: only the near plane shows it.
    let (center, radius) = (Vec3::new(9.0, 1.0, -2.0), 0.5);
    assert!(sees(&graph, eye, target, center, radius));
    let mut traced = TracedView::default();
    let frustum = planes(&camera(eye, target));
    let viewer = Viewer {
        eye,
        near: &[],
        frustum: &frustum,
    };
    let point_eye = trace(
        &graph,
        viewer,
        &[],
        16,
        &mut traced,
        &mut TraceCounts::default(),
    );
    assert!(point_eye);
    assert!(!traced.sees(
        |node| graph.reaches(node, center, radius),
        center,
        radius,
        TOLERANCE,
    ));
}

#[test]
fn a_camera_facing_an_outer_wall_starts_outside_too() {
    let graph = row_graph(2, &[0.0]);
    let eye = Vec3::new(0.05, 1.0, 1.0);
    assert_eq!(starts_of(&graph, eye, eye - Vec3::X), [OUTSIDE, 1]);
    // From outside, facing the wall: the room behind it starts too.
    let eye = Vec3::new(-0.05, 1.0, 1.0);
    assert_eq!(starts_of(&graph, eye, eye + Vec3::X), [OUTSIDE, 1]);
}

#[test]
fn a_pyramid_across_a_doorway_stays_inside() {
    let graph = row_graph(2, &[0.0]);
    let eye = Vec3::new(5.95, 1.0, 0.0);
    assert_eq!(starts_of(&graph, eye, eye + Vec3::X), [1, 2]);
    // Looking along the wall across the doorway, the pyramid stays in the two rooms.
    assert_eq!(
        starts_of(&graph, eye, eye + Vec3::new(1.0, 0.0, 1.0)),
        [1, 2]
    );
}

/// Rooms 6 m wide in the quadrants around `(6, _, 0)`: all four, or three with the `+X, −Z`
/// quadrant outside.
fn corner(four: bool) -> Graph {
    let size = Vec3::new(6.0, 3.0, 6.0);
    let mut rooms = vec![
        room(Vec3::new(3.0, 1.5, 3.0), size),
        room(Vec3::new(9.0, 1.5, 3.0), size),
        room(Vec3::new(3.0, 1.5, -3.0), size),
    ];
    if four {
        rooms.push(room(Vec3::new(9.0, 1.5, -3.0), size));
    }
    Graph::build(&rooms, &[], TOLERANCE).unwrap()
}

#[test]
fn a_pyramid_at_a_corner_starts_outside_only_where_no_room_is() {
    let eye = Vec3::new(5.96, 1.0, 0.04);
    let target = eye + Vec3::new(1.0, 0.0, -1.0);
    assert_eq!(starts_of(&corner(true), eye, target), [1, 2, 3, 4]);
    assert_eq!(starts_of(&corner(false), eye, target), [OUTSIDE, 1, 2, 3]);
}

#[test]
fn far_from_the_origin_the_starts_are_the_same() {
    let offset = Vec3::new(10_000.0, 0.0, -10_000.0);
    let rooms = [
        room(Vec3::new(3.0, 1.5, 0.0) + offset, Vec3::new(6.0, 3.0, 6.0)),
        room(Vec3::new(9.0, 1.5, 0.0) + offset, Vec3::new(6.0, 3.0, 6.0)),
    ];
    let graph = Graph::build(&rooms, &[], TOLERANCE).unwrap();
    let eye = Vec3::new(5.95, 1.0, -2.0) + offset;
    assert_eq!(starts_of(&graph, eye, eye + Vec3::X), [1, 2]);
    assert_eq!(starts_of(&graph, eye, eye - Vec3::X), [1]);
    let eye = Vec3::new(0.05, 1.0, 1.0) + offset;
    assert_eq!(starts_of(&graph, eye, eye - Vec3::X), [OUTSIDE, 1]);
}

#[test]
fn a_flat_or_missing_near_plane_starts_at_the_eye() {
    let graph = row_graph(2, &[0.0]);
    let eye = Vec3::new(5.95, 1.0, -2.0);
    let mut out = Vec::new();
    starts(&graph, eye, &[], &mut out);
    assert_eq!(out, [1]);
    starts(&graph, eye, &[eye; 4], &mut out);
    assert_eq!(out, [1]);
}
