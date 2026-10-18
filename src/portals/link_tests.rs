//! Cell geometry, validation and linking.

use bevy::math::{Affine3A, Mat3, Quat, Vec2, Vec3};

use super::OUTSIDE;
use super::cell::{Aperture, Cell, Parallelepiped, distance, ray_span};
use super::fixtures::{TOLERANCE, door, portal, room, row, row_graph};
use super::graph::{Graph, PortalInput, Problem, RoomInput, SideInput};
use super::link::{Unresolved, covers, resolve};

fn sheared() -> Affine3A {
    let shear = Mat3::from_cols(
        Vec3::new(2.0, 0.0, 0.0),
        Vec3::new(0.8, 1.0, 0.0),
        Vec3::new(0.0, 0.3, 0.5),
    );
    Affine3A::from_mat3_translation(
        Mat3::from_quat(Quat::from_rotation_y(0.6)) * shear,
        Vec3::new(4.0, -1.0, 2.0),
    )
}

#[test]
fn cells_hold_exactly_the_image_of_their_box() {
    let affine = sheared();
    let cell = Cell::new(Parallelepiped::from_affine(affine, Vec3::new(1.0, 2.0, 0.5)).unwrap());
    for corner in cell.shape.corners() {
        let on_faces = cell
            .planes
            .iter()
            .filter(|&&plane| distance(plane, corner).abs() < 1e-4)
            .count();
        assert_eq!(on_faces, 3, "{corner}");
        assert!(cell.depth(corner) > -1e-4);
    }
    for local in [
        Vec3::new(0.9, 1.9, 0.4),
        Vec3::new(-0.5, 0.1, -0.45),
        Vec3::ZERO,
    ] {
        assert!(cell.depth(affine.transform_point3(local)) > 0.0, "{local}");
    }
    for local in [Vec3::new(1.1, 0.0, 0.0), Vec3::new(0.0, -2.1, 0.0)] {
        assert!(cell.depth(affine.transform_point3(local)) < 0.0, "{local}");
    }
}

#[test]
fn flat_singular_or_broken_boxes_are_refused() {
    let flat = Affine3A::from_scale(Vec3::new(1.0, 0.0, 1.0));
    assert!(Parallelepiped::from_affine(flat, Vec3::ONE).is_none());
    assert!(Parallelepiped::from_affine(Affine3A::IDENTITY, Vec3::new(1.0, 1.0, 0.0)).is_none());
    let broken = Affine3A::from_translation(Vec3::new(f32::NAN, 0.0, 0.0));
    assert!(Parallelepiped::from_affine(broken, Vec3::ONE).is_none());
    assert!(Aperture::from_affine(Affine3A::IDENTITY, Vec2::new(1.0, 0.0)).is_none());
    let rooms = [room(Vec3::ZERO, Vec3::ZERO)];
    assert_eq!(Graph::build(&rooms, &[], TOLERANCE), Err(Problem::Room(0)));
}

#[test]
fn overlap_is_the_depth_along_the_best_separating_axis() {
    let a = Parallelepiped::from_affine(Affine3A::IDENTITY, Vec3::ONE).unwrap();
    let at = |x: f32| {
        Parallelepiped::from_affine(
            Affine3A::from_translation(Vec3::new(x, 0.0, 0.0)),
            Vec3::ONE,
        )
        .unwrap()
    };
    assert!((a.overlap(&at(2.5)) + 0.5).abs() < 1e-5);
    assert!(a.overlap(&at(2.0)).abs() < 1e-5);
    assert!((a.overlap(&at(1.9)) - 0.1).abs() < 1e-5);
    // Turned about two axes, the boxes' bounds overlap but an axis between their edges or
    // faces still separates them.
    let turned = Affine3A::from_rotation_translation(
        Quat::from_rotation_y(0.785) * Quat::from_rotation_x(0.785),
        Vec3::new(2.2, 2.2, 0.0),
    );
    let tilted = Parallelepiped::from_affine(turned, Vec3::ONE).unwrap();
    let (low, high) = tilted.bounds();
    assert!(
        low.x < 1.0 && low.y < 1.0 && high.z > -1.0,
        "the bounds overlap"
    );
    assert!(a.overlap(&tilted) < 0.0);
}

#[test]
fn ray_spans_cross_the_planes_once() {
    let cell = Cell::new(Parallelepiped::from_affine(Affine3A::IDENTITY, Vec3::ONE).unwrap());
    let (low, high) = ray_span(&cell.planes, Vec3::new(-5.0, 0.0, 0.0), Vec3::X).unwrap();
    assert!((low - 4.0).abs() < 1e-5 && (high - 6.0).abs() < 1e-5);
    assert!(ray_span(&cell.planes, Vec3::new(-5.0, 2.0, 0.0), Vec3::X).is_none());
}

/// The sides of every portal of `rooms` and `portals`.
fn sides(rooms: &[RoomInput], portals: &[PortalInput], tolerance: f32) -> Vec<[Vec<usize>; 2]> {
    let graph = Graph::build(rooms, portals, tolerance).expect("valid");
    graph
        .portals
        .iter()
        .map(|portal| portal.sides.clone())
        .collect()
}

#[test]
fn a_cell_that_reaches_the_portal_takes_that_side() {
    let (rooms, portals) = row(2, &[0.0]);
    assert_eq!(sides(&rooms, &portals, TOLERANCE), [[vec![1], vec![2]]]);
    let graph = row_graph(2, &[0.0]);
    assert_eq!(graph.links(1)[0].to, 2);
    assert!(graph.links(1)[0].outward.abs_diff_eq(Vec3::X, 1e-6));
    assert!(graph.links(2)[0].outward.abs_diff_eq(Vec3::NEG_X, 1e-6));
}

#[test]
fn a_window_with_no_cell_near_it_opens_outside() {
    let (rooms, _) = row(1, &[]);
    let window = door(6.0, 1.0);
    assert_eq!(
        sides(&rooms, &[window], TOLERANCE),
        [[vec![1], vec![OUTSIDE]]]
    );
}

#[test]
fn a_cell_short_of_the_portal_is_not_taken_for_the_outside() {
    // The far room starts 0.55 m past the door: probes at 0.5 m and 1 m would both miss it.
    let rooms = [
        room(Vec3::new(3.0, 1.5, 0.0), Vec3::new(6.0, 3.0, 6.0)),
        room(Vec3::new(6.75, 1.5, 0.0), Vec3::new(0.4, 3.0, 6.0)),
    ];
    let problem = Graph::build(&rooms, &[door(6.0, 0.0)], TOLERANCE);
    let expected = Problem::Side {
        portal: 0,
        side: 1,
        issue: Unresolved::Uncovered(vec![1]),
    };
    assert_eq!(problem, Err(expected));
}

#[test]
fn the_tolerance_is_measured_not_sampled() {
    // A room 0.3 m past the door: inside a 0.4 m tolerance it takes the side, inside 0.02 m
    // it is a near miss.
    let rooms = [
        room(Vec3::new(3.0, 1.5, 0.0), Vec3::new(6.0, 3.0, 6.0)),
        room(Vec3::new(9.3, 1.5, 0.0), Vec3::new(6.0, 3.0, 6.0)),
    ];
    assert_eq!(sides(&rooms, &[door(6.0, 0.0)], 0.4), [[vec![1], vec![2]]]);
    assert!(Graph::build(&rooms, &[door(6.0, 0.0)], TOLERANCE).is_err());
}

#[test]
fn an_opening_onto_two_cells_waits_for_an_explicit_side() {
    // A 2 m doorway whose middle opens into a booth and whose edges open past it into a hall.
    let rooms = [
        room(Vec3::new(-3.0, 1.5, 0.0), Vec3::new(6.0, 3.0, 6.0)),
        room(Vec3::new(0.4, 1.5, 0.0), Vec3::new(0.8, 3.0, 0.6)),
        room(Vec3::new(3.9, 1.5, 0.0), Vec3::new(6.2, 3.0, 6.0)),
    ];
    let doorway = portal(Vec3::new(0.0, 1.0, 0.0), Vec2::new(2.0, 2.0), Vec3::X);
    let problem = Graph::build(&rooms, core::slice::from_ref(&doorway), TOLERANCE);
    assert!(
        matches!(&problem, Err(Problem::Side { side: 1, issue: Unresolved::Uncovered(cells), .. })
            if cells == &[1, 2]),
        "{problem:?}"
    );
    let listed = PortalInput {
        sides: [
            SideInput::Auto,
            SideInput::Into {
                rooms: vec![1, 2],
                outside: true,
            },
        ],
        ..doorway
    };
    assert_eq!(
        sides(&rooms, &[listed], TOLERANCE),
        [[vec![1], vec![0, 2, 3]]]
    );
}

#[test]
fn a_cell_reaching_back_through_the_portal_does_not_cover_it() {
    let cell = Cell::new(
        Parallelepiped::from_affine(
            Affine3A::from_translation(Vec3::new(2.975, 1.5, 0.0)),
            Vec3::new(3.025, 1.5, 3.0),
        )
        .unwrap(),
    );
    let aperture = Aperture::from_affine(
        Affine3A::from_rotation_translation(
            Quat::from_rotation_arc(Vec3::Z, Vec3::X),
            Vec3::new(0.0, 1.0, 0.0),
        ),
        Vec2::new(1.0, 2.0),
    )
    .unwrap();
    assert!(!covers(&cell, &aperture, 1.0, TOLERANCE));
    let cells = [cell];
    assert!(resolve(&aperture, 1.0, &cells, TOLERANCE).is_err());
    assert!(resolve(&aperture, -1.0, &cells, TOLERANCE).is_err());
}

#[test]
fn overlapping_rooms_fail_the_build_and_touching_ones_do_not() {
    let rooms = [
        room(Vec3::new(3.0, 1.5, 0.0), Vec3::new(6.0, 3.0, 6.0)),
        room(Vec3::new(8.9, 1.5, 0.0), Vec3::new(6.0, 3.0, 6.0)),
    ];
    assert!(matches!(
        Graph::build(&rooms, &[], TOLERANCE),
        Err(Problem::Overlap(0, 1, depth)) if (depth - 0.1).abs() < 1e-4
    ));
    let (touching, _) = row(3, &[]);
    assert!(Graph::build(&touching, &[], TOLERANCE).is_ok());
}

#[test]
fn explicit_sides_must_name_rooms() {
    let (rooms, _) = row(2, &[]);
    for side in [
        SideInput::Into {
            rooms: vec![],
            outside: false,
        },
        SideInput::Into {
            rooms: vec![7],
            outside: false,
        },
    ] {
        let portal = PortalInput {
            sides: [SideInput::Auto, side],
            ..door(6.0, 0.0)
        };
        let expected = Problem::Into { portal: 0, side: 1 };
        assert_eq!(Graph::build(&rooms, &[portal], TOLERANCE), Err(expected));
    }
}

#[test]
fn a_portal_with_one_node_on_both_sides_joins_nothing() {
    let (rooms, _) = row(2, &[]);
    let inside = door(3.0, 0.0);
    let graph = Graph::build(&rooms, core::slice::from_ref(&inside), TOLERANCE);
    assert!(
        graph.is_err(),
        "the room straddles it, so its sides are unclear"
    );
    let both = SideInput::Into {
        rooms: vec![0],
        outside: false,
    };
    let listed = PortalInput {
        sides: [both.clone(), both],
        ..inside
    };
    let graph = Graph::build(&rooms, &[listed], TOLERANCE).unwrap();
    assert!(graph.portals[0].is_ignored());
    assert!(graph.links(1).is_empty());
}

#[test]
fn the_order_rooms_and_portals_come_in_changes_no_link() {
    let (rooms, portals) = row(4, &[0.0, 1.5, -2.0]);
    let forward = sides(&rooms, &portals, TOLERANCE);
    let reversed_rooms: Vec<_> = rooms.iter().rev().copied().collect();
    let reversed_portals: Vec<_> = portals.iter().rev().cloned().collect();
    let backward = sides(&reversed_rooms, &reversed_portals, TOLERANCE);
    let renumber = |node: usize| if node == OUTSIDE { OUTSIDE } else { 5 - node };
    for (index, sides) in forward.iter().enumerate() {
        let other = &backward[forward.len() - 1 - index];
        for side in 0..2 {
            let mut mapped: Vec<_> = other[side].iter().map(|&node| renumber(node)).collect();
            mapped.sort_unstable();
            assert_eq!(mapped, sides[side]);
        }
    }
}

#[test]
fn points_are_home_in_every_cell_near_them_and_outside_past_them() {
    let graph = row_graph(2, &[0.0]);
    let mut homes = Vec::new();
    graph.homes(Vec3::new(2.0, 1.0, 0.0), &mut homes);
    assert_eq!(homes, [1]);
    graph.homes(Vec3::new(6.0, 1.0, 0.0), &mut homes);
    assert_eq!(homes, [1, 2]);
    graph.homes(Vec3::new(-0.01, 1.0, 0.0), &mut homes);
    assert_eq!(homes, [1, OUTSIDE]);
    graph.homes(Vec3::new(-5.0, 1.0, 0.0), &mut homes);
    assert_eq!(homes, [OUTSIDE]);
}

#[test]
fn spheres_reach_through_walls() {
    let graph = row_graph(2, &[0.0]);
    let mut reach = Vec::new();
    graph.reach(Vec3::new(3.0, 1.5, 0.0), 1.0, &mut reach);
    assert_eq!(reach, [1]);
    // A sphere against a solid wall still reaches the room behind it, and the outside.
    graph.reach(Vec3::new(5.0, 1.5, 2.5), 1.5, &mut reach);
    assert_eq!(reach, [1, 2, OUTSIDE]);
    // Just touching the wall counts for the room behind it; the first room still holds it.
    graph.reach(Vec3::new(5.0, 1.5, 0.0), 1.0, &mut reach);
    assert_eq!(reach, [1, 2]);
    graph.reach(Vec3::new(-3.0, 1.5, 0.0), 1.0, &mut reach);
    assert_eq!(reach, [OUTSIDE]);
}

#[test]
fn one_node_at_a_time_reaches_the_same_nodes() {
    let graph = row_graph(3, &[0.0, 1.0]);
    let mut reach = Vec::new();
    for i in 0..400u32 {
        // A lattice over the rooms and past them, with radii from 0.1 to 3 m.
        let center = Vec3::new(
            (i % 20) as f32 - 2.0,
            (i % 7) as f32 * 0.6 - 0.5,
            ((i / 20) % 10) as f32 - 4.5,
        );
        let radius = 0.1 + (i % 11) as f32 * 0.29;
        graph.reach(center, radius, &mut reach);
        for node in 0..graph.node_count() {
            assert_eq!(
                graph.reaches(node, center, radius),
                reach.contains(&node),
                "node {node}, sphere at {center} r {radius}"
            );
        }
    }
}
