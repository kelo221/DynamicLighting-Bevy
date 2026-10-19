//! Closed portals in the trace: a shut door hides what lies only past it, never a light whose
//! sphere reaches the view some other way, and opening it again shows everything it did before.

use bevy::math::Vec3;

use super::fixtures::{
    TOLERANCE, camera, door, near, planes, row, row_graph, sees_closed, view_closed,
};
use super::graph::Graph;
use super::trace::{TraceCounts, TracedView, Viewer, trace};

const EYE: Vec3 = Vec3::new(2.0, 1.0, 0.0);
const TARGET: Vec3 = Vec3::new(9.0, 1.0, 0.0);
/// In the second room of a row, straight through the door at `x = 6`.
const THROUGH: Vec3 = Vec3::new(9.0, 1.0, 0.0);

#[test]
fn a_closed_door_hides_the_next_room() {
    let graph = row_graph(2, &[0.0]);
    assert!(sees_closed(&graph, &[false], EYE, TARGET, THROUGH, 0.5));
    assert!(!sees_closed(&graph, &[true], EYE, TARGET, THROUGH, 0.5));
    let view = view_closed(&graph, EYE, TARGET, &[true], 16).0.unwrap();
    assert!(view.visits(1) && !view.visits(2));
}

#[test]
fn a_light_reaching_through_the_wall_beside_a_closed_door_stays() {
    let graph = row_graph(2, &[0.0]);
    // In the second room, its sphere crossing the wall into the first room's view.
    let center = Vec3::new(6.5, 1.0, -1.0);
    assert!(sees_closed(&graph, &[true], EYE, TARGET, center, 1.0));
    assert!(!sees_closed(&graph, &[true], EYE, TARGET, center, 0.4));
}

#[test]
fn reopening_a_door_shows_the_room_again() {
    let graph = row_graph(2, &[0.0]);
    let (corners, frustum) = (near(EYE, TARGET), planes(&camera(EYE, TARGET)));
    let viewer = Viewer {
        eye: EYE,
        near: &corners,
        frustum: &frustum,
    };
    let mut traced = TracedView::default();
    let mut runs = Vec::new();
    for closed in [false, true, false] {
        assert!(trace(
            &graph,
            viewer,
            &[closed],
            16,
            &mut traced,
            &mut TraceCounts::default(),
        ));
        runs.push(traced.clone());
    }
    assert!(runs[0].visits(2) && !runs[1].visits(2));
    assert_eq!(runs[0], runs[2]);
}

#[test]
fn a_closed_window_hides_the_outside_and_the_room_from_outside() {
    let (rooms, _) = row(1, &[]);
    let graph = Graph::build(&rooms, &[door(6.0, 0.0)], TOLERANCE).unwrap();
    let outside = Vec3::new(9.0, 1.0, 0.0);
    assert!(sees_closed(&graph, &[false], EYE, TARGET, outside, 0.5));
    assert!(!sees_closed(&graph, &[true], EYE, TARGET, outside, 0.5));
    let inside = Vec3::new(3.0, 1.0, 0.0);
    assert!(sees_closed(&graph, &[false], outside, EYE, inside, 0.5));
    assert!(!sees_closed(&graph, &[true], outside, EYE, inside, 0.5));
}

#[test]
fn an_eye_at_a_closed_door_sees_past_it_only_within_the_near_plane() {
    let graph = row_graph(2, &[0.0]);
    // On the door's plane, and 5 cm before it: the near plane clips the door leaf.
    for x in [6.0, 5.95] {
        let eye = Vec3::new(x, 1.0, 0.0);
        assert!(
            sees_closed(&graph, &[true], eye, TARGET, THROUGH, 0.5),
            "{x}"
        );
    }
    let eye = Vec3::new(5.5, 1.0, 0.0);
    assert!(!sees_closed(&graph, &[true], eye, TARGET, THROUGH, 0.5));
}

#[test]
fn of_two_doorways_only_the_closed_one_stops_showing_the_room() {
    let (rooms, _) = row(2, &[]);
    let doors = [door(6.0, -1.5), door(6.0, 1.5)];
    let graph = Graph::build(&rooms, &doors, TOLERANCE).unwrap();
    // Each in view through one doorway only.
    let through_open = Vec3::new(9.0, 1.0, -2.6);
    let through_closed = Vec3::new(9.0, 1.0, 2.6);
    for center in [through_open, through_closed] {
        assert!(sees_closed(
            &graph,
            &[false, false],
            EYE,
            TARGET,
            center,
            0.3
        ));
    }
    assert!(sees_closed(
        &graph,
        &[false, true],
        EYE,
        TARGET,
        through_open,
        0.3
    ));
    assert!(!sees_closed(
        &graph,
        &[false, true],
        EYE,
        TARGET,
        through_closed,
        0.3
    ));
}

#[test]
fn closing_a_door_can_bound_a_view_past_the_depth_limit() {
    let graph = row_graph(5, &[0.0; 4]);
    let (eye, target) = (Vec3::new(1.0, 1.0, 0.0), Vec3::new(29.0, 1.0, 0.0));
    assert!(view_closed(&graph, eye, target, &[], 2).0.is_none());
    let closed = [false, false, true, false];
    let view = view_closed(&graph, eye, target, &closed, 2).0.unwrap();
    assert!(view.visits(3) && !view.visits(4));
}
