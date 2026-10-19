//! Tracing views through portals, and testing spheres against them.

use bevy::math::{Affine3A, Vec2, Vec3};

use super::cell::distance;
use super::fixtures::{TOLERANCE, door, portal, room, row, row_graph, sees, view, view_with};
use super::graph::{Graph, PortalInput, RoomInput};
use super::trace::VISIT_LIMIT;

const LEVEL: f32 = 1.5;

fn at(x: f32, z: f32) -> Vec3 {
    Vec3::new(x, LEVEL, z)
}

#[test]
fn a_door_in_view_shows_the_next_room_and_one_behind_does_not() {
    let graph = row_graph(2, &[0.0]);
    let ahead = view(&graph, at(2.0, 0.0), at(8.0, 0.0)).unwrap();
    assert!(ahead.visits(1) && ahead.visits(2));
    let behind = view(&graph, at(2.0, 0.0), at(-4.0, 0.0)).unwrap();
    assert!(behind.visits(1) && !behind.visits(2));
    // Through the door the eye sees `|z| ≤ 1` at `x = 9`: a light there is kept, one beside it
    // behind the wall is culled though the frustum holds it.
    let (eye, target) = (at(3.0, 0.0), at(9.0, 0.0));
    assert!(sees(&graph, eye, target, at(9.0, 0.0), 0.5));
    let beside = at(9.0, 2.0);
    assert!(crate::shared::sees_sphere(
        &super::fixtures::camera(eye, target),
        beside,
        0.5
    ));
    assert!(!sees(&graph, eye, target, beside, 0.5));
}

#[test]
fn aligned_doors_show_the_third_room_and_offset_ones_hide_it() {
    let aligned = row_graph(3, &[0.0, 0.0]);
    let eye = at(1.0, 0.0);
    assert!(view(&aligned, eye, at(15.0, 0.0)).unwrap().visits(3));
    assert!(sees(&aligned, eye, at(15.0, 0.0), at(15.0, 0.0), 0.5));
    let offset = row_graph(3, &[0.0, 2.5]);
    assert!(!view(&offset, eye, at(15.0, 0.0)).unwrap().visits(3));
    assert!(!sees(&offset, eye, at(15.0, 0.0), at(15.0, 0.0), 1.0));
}

#[test]
fn an_eye_in_a_doorway_or_just_past_it_sees_both_rooms() {
    let graph = row_graph(2, &[0.0]);
    for x in [6.0, 5.99, 6.01] {
        let forward = view(&graph, Vec3::new(x, 1.0, 0.0), at(12.0, 0.0)).unwrap();
        assert!(forward.visits(2), "{x}");
        let back = view(&graph, Vec3::new(x, 1.0, 0.0), at(0.0, 0.0)).unwrap();
        assert!(back.visits(1), "{x}");
        assert!(sees(
            &graph,
            Vec3::new(x, 1.0, 0.0),
            at(0.0, 0.0),
            at(1.0, 2.0),
            0.5
        ));
    }
    let past = view(&graph, Vec3::new(6.05, 1.0, 0.0), at(0.0, 0.0)).unwrap();
    assert!(past.visits(1));
}

/// A corridor of two rooms along X with a room beside each on +Z, both side doors in the
/// plane `z = 3`.
fn side_rooms() -> Graph {
    let (mut rooms, mut portals) = row(2, &[0.0]);
    let size = Vec3::new(6.0, 3.0, 6.0);
    rooms.push(room(Vec3::new(3.0, 1.5, 6.0), size));
    rooms.push(room(Vec3::new(9.0, 1.5, 6.0), size));
    portals.push(portal(
        Vec3::new(3.0, 1.0, 3.0),
        Vec2::new(1.0, 2.0),
        Vec3::Z,
    ));
    portals.push(portal(
        Vec3::new(9.0, 1.0, 3.0),
        Vec2::new(1.0, 2.0),
        Vec3::Z,
    ));
    Graph::build(&rooms, &portals, TOLERANCE).unwrap()
}

#[test]
fn portals_in_the_eyes_plane_still_show_what_is_behind_them() {
    let graph = side_rooms();
    // The eye stands in the plane of both side doors, looking at the door to the second room.
    let eye = Vec3::new(3.0, 1.0, 2.995);
    let along = view(&graph, eye, Vec3::new(6.0, 1.0, 0.0)).unwrap();
    for node in 1..=4 {
        assert!(along.visits(node), "{node}");
    }
}

#[test]
fn a_door_seen_edge_on_keeps_finite_planes() {
    let graph = row_graph(2, &[0.0]);
    let eye = Vec3::new(6.03, 1.0, -2.0);
    let traced = view(&graph, eye, at(6.0, 3.0)).unwrap();
    for (_, planes) in traced.visit_planes() {
        assert!(planes.iter().all(|plane| plane.is_finite()));
    }
}

#[test]
fn two_doorways_into_one_room_are_two_views_of_it() {
    let (rooms, _) = row(2, &[]);
    let portals = [door(6.0, -2.0), door(6.0, 2.0)];
    let graph = Graph::build(&rooms, &portals, TOLERANCE).unwrap();
    let (eye, target) = (at(1.0, 0.0), at(8.0, 0.0));
    let traced = view(&graph, eye, target).unwrap();
    assert_eq!(
        traced.visit_planes().filter(|(node, _)| *node == 2).count(),
        2
    );
    // Seen through the second door only, then through neither.
    assert!(sees(&graph, eye, target, at(8.0, 2.6), 0.2));
    assert!(!sees(&graph, eye, target, at(8.0, 0.0), 0.2));
}

#[test]
fn a_view_never_steps_back_through_the_door_it_came_by() {
    let graph = row_graph(2, &[0.0]);
    let traced = view(&graph, at(2.0, 0.0), at(8.0, 0.0)).unwrap();
    assert_eq!(traced.visit_count(), 2);
}

#[test]
fn a_view_deeper_than_its_limit_culls_nothing() {
    let graph = row_graph(20, &[0.0; 19]);
    let (eye, target) = (Vec3::new(1.0, 1.0, 0.0), Vec3::new(100.0, 1.0, 0.0));
    assert!(view_with(&graph, eye, target, 16).0.is_none());
    let deep = view_with(&graph, eye, target, 32).0.unwrap();
    assert!(deep.visits(20));
}

/// Two rooms joined by `count` copies of one door, and a third joined to the second the same
/// way.
fn many_doors(count: usize, rooms: usize) -> Graph {
    let (rooms, _) = row(rooms, &[]);
    let mut portals: Vec<PortalInput> = (0..count).map(|_| door(6.0, 0.0)).collect();
    if rooms.len() > 2 {
        portals.extend((0..count).map(|_| door(12.0, 0.0)));
    }
    Graph::build(&rooms, &portals, TOLERANCE).unwrap()
}

#[test]
fn a_view_past_the_work_limits_culls_nothing() {
    // 1 + 70 + 70 · 70 visits.
    let (eye, ahead) = (Vec3::new(1.0, 1.0, 0.0), Vec3::new(15.0, 1.0, 0.0));
    let (traced, counts) = view_with(&many_doors(70, 3), eye, ahead, 16);
    assert!(traced.is_none() && counts.visits > VISIT_LIMIT);
    let (traced, counts) = view_with(&many_doors(20, 3), eye, ahead, 16);
    assert!(traced.is_some(), "{counts:?}");
    // Every door is tested, even those behind the eye.
    let (traced, counts) = view_with(&many_doors(66_000, 2), eye, -ahead, 16);
    assert!(traced.is_none() && counts.visits == 1, "{counts:?}");
}

#[test]
fn every_plane_bounds_the_eye_from_outside() {
    let graph = side_rooms();
    for (eye, target) in [
        (at(1.0, 0.0), at(11.0, 0.0)),
        (at(1.0, -2.0), at(9.0, 5.0)),
        (at(10.0, 2.0), at(2.0, -1.0)),
        (Vec3::new(9.0, 1.0, 8.0), Vec3::new(4.0, 1.0, 0.0)),
    ] {
        let traced = view(&graph, eye, target).unwrap();
        for (node, planes) in traced.visit_planes() {
            for &plane in planes {
                let at_eye = distance(plane, Vec3::ZERO);
                assert!(at_eye <= TOLERANCE + 1e-4, "{node}: {plane} at {at_eye}");
            }
        }
    }
}

#[test]
fn far_from_the_origin_the_answers_stay_the_same() {
    let offset = Vec3::new(10_000.0, 0.0, -10_000.0);
    let (rooms, portals) = row(3, &[0.0, 1.0]);
    let moved_rooms: Vec<RoomInput> = rooms
        .iter()
        .map(|input| RoomInput {
            affine: Affine3A::from_translation(offset) * input.affine,
            ..*input
        })
        .collect();
    let moved_portals: Vec<PortalInput> = portals
        .iter()
        .map(|input| PortalInput {
            affine: Affine3A::from_translation(offset) * input.affine,
            ..input.clone()
        })
        .collect();
    let near = Graph::build(&rooms, &portals, TOLERANCE).unwrap();
    let far = Graph::build(&moved_rooms, &moved_portals, TOLERANCE).unwrap();
    let (eye, target) = (at(2.0, -1.0), at(14.0, 1.0));
    for x in [2.0, 7.0, 9.0, 13.0, 16.0] {
        for z in [-2.5, 0.0, 2.5] {
            let center = at(x, z);
            assert_eq!(
                sees(&near, eye, target, center, 0.4),
                sees(&far, eye + offset, target + offset, center + offset, 0.4),
                "{center}"
            );
        }
    }
}

#[test]
fn with_no_rooms_everything_is_outside_and_seen() {
    let graph = Graph::build(&[], &[], TOLERANCE).unwrap();
    let traced = view(&graph, at(0.0, 0.0), at(10.0, 0.0)).unwrap();
    assert_eq!(traced.visit_count(), 1);
    assert!(sees(&graph, at(0.0, 0.0), at(10.0, 0.0), at(9.0, 1.0), 0.5));
    assert!(!sees(
        &graph,
        at(0.0, 0.0),
        at(10.0, 0.0),
        at(-9.0, 1.0),
        0.5
    ));
}

#[test]
fn an_eye_near_plane_or_frustum_that_is_not_finite_culls_nothing() {
    use super::fixtures::{camera, planes};
    use super::trace::{TraceCounts, TracedView, Viewer, trace};
    let graph = row_graph(2, &[0.0]);
    let frustum = planes(&camera(at(2.0, 0.0), at(8.0, 0.0)));
    let mut broken = frustum;
    broken[2].x = f32::NAN;
    let near = [Vec3::NAN; 4];
    for (eye, near, frustum) in [
        (Vec3::NAN, &[][..], frustum),
        (at(2.0, 0.0), &[], broken),
        (at(2.0, 0.0), &near, frustum),
    ] {
        let mut traced = TracedView::default();
        let mut counts = TraceCounts::default();
        let viewer = Viewer {
            eye,
            near,
            frustum: &frustum,
        };
        assert!(!trace(&graph, viewer, &[], 16, &mut traced, &mut counts));
    }
}
