//! Rooms, portals and cameras for the portal tests.

use bevy::camera::primitives::Frustum;
use bevy::camera::{CameraProjection, PerspectiveProjection};
use bevy::math::{Affine3A, Quat, Vec2, Vec3, Vec4};
use bevy::transform::components::{GlobalTransform, Transform};

use super::graph::{Graph, PortalInput, RoomInput, SideInput};
use super::near::near_corners;
use super::trace::{TraceCounts, TracedView, Viewer, trace};

pub const TOLERANCE: f32 = 0.02;

/// A cell `size` across around `center`.
pub fn room(center: Vec3, size: Vec3) -> RoomInput {
    RoomInput {
        affine: Affine3A::from_translation(center),
        half: size * 0.5,
    }
}

/// An automatically linked `size` opening at `center` whose front faces `facing`.
pub fn portal(center: Vec3, size: Vec2, facing: Vec3) -> PortalInput {
    let rotation = Quat::from_rotation_arc(Vec3::Z, facing.normalize());
    PortalInput {
        affine: Affine3A::from_rotation_translation(rotation, center),
        size,
        sides: [SideInput::Auto, SideInput::Auto],
    }
}

/// A 1 m × 2 m door in the wall `x = x` at `z`, its front facing +X.
pub fn door(x: f32, z: f32) -> PortalInput {
    portal(Vec3::new(x, 1.0, z), Vec2::new(1.0, 2.0), Vec3::X)
}

/// `count` rooms 6 m long in a row along +X from `x = 0`, 3 m high, 6 m deep around `z = 0`,
/// each joined to the next by a door at `z = doors[i]`.
pub fn row(count: usize, doors: &[f32]) -> (Vec<RoomInput>, Vec<PortalInput>) {
    let rooms = (0..count)
        .map(|index| {
            room(
                Vec3::new(6.0 * index as f32 + 3.0, 1.5, 0.0),
                Vec3::new(6.0, 3.0, 6.0),
            )
        })
        .collect();
    let portals = doors
        .iter()
        .enumerate()
        .map(|(index, &z)| door(6.0 * (index + 1) as f32, z))
        .collect();
    (rooms, portals)
}

/// The graph of `row(count, doors)`.
pub fn row_graph(count: usize, doors: &[f32]) -> Graph {
    let (rooms, portals) = row(count, doors);
    Graph::build(&rooms, &portals, TOLERANCE).expect("the row is valid")
}

/// A camera's world planes at `eye` looking at `target` (45° field of view, square, near plane
/// 0.1 m away).
pub fn camera(eye: Vec3, target: Vec3) -> Frustum {
    let transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
    PerspectiveProjection::default().compute_frustum(&GlobalTransform::from(transform))
}

/// The near plane's corners of [`camera`].
pub fn near(eye: Vec3, target: Vec3) -> [Vec3; 4] {
    let transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
    let clip_from_view = PerspectiveProjection::default().get_clip_from_view();
    near_corners(clip_from_view, transform.compute_affine()).expect("the camera is valid")
}

pub fn planes(frustum: &Frustum) -> [Vec4; 6] {
    frustum.half_spaces.map(|half_space| half_space.normal_d())
}

/// The view from `eye` towards `target`, or `None` when it is unbounded.
pub fn view(graph: &Graph, eye: Vec3, target: Vec3) -> Option<TracedView> {
    view_with(graph, eye, target, 16).0
}

/// The view and the trace's work, with `depth_limit`.
pub fn view_with(
    graph: &Graph,
    eye: Vec3,
    target: Vec3,
    depth_limit: u32,
) -> (Option<TracedView>, TraceCounts) {
    view_closed(graph, eye, target, &[], depth_limit)
}

/// [`view_with`] with the portals set in `closed` shut.
pub fn view_closed(
    graph: &Graph,
    eye: Vec3,
    target: Vec3,
    closed: &[bool],
    depth_limit: u32,
) -> (Option<TracedView>, TraceCounts) {
    let mut traced = TracedView::default();
    let mut counts = TraceCounts::default();
    let (near, frustum) = (near(eye, target), planes(&camera(eye, target)));
    let viewer = Viewer {
        eye,
        near: &near,
        frustum: &frustum,
    };
    let bounded = trace(graph, viewer, closed, depth_limit, &mut traced, &mut counts);
    (bounded.then_some(traced), counts)
}

/// Whether `view` (with the frustum from `eye` towards `target`) may see the sphere.
pub fn sees(graph: &Graph, eye: Vec3, target: Vec3, center: Vec3, radius: f32) -> bool {
    sees_closed(graph, &[], eye, target, center, radius)
}

/// [`sees`] with the portals set in `closed` shut.
pub fn sees_closed(
    graph: &Graph,
    closed: &[bool],
    eye: Vec3,
    target: Vec3,
    center: Vec3,
    radius: f32,
) -> bool {
    let frustum = camera(eye, target);
    if !crate::shared::sees_sphere(&frustum, center, radius) {
        return false;
    }
    let Some(view) = view_closed(graph, eye, target, closed, 16).0 else {
        return true;
    };
    view.sees(
        |node| graph.reaches(node, center, radius),
        center,
        radius,
        graph.tolerance,
    )
}
