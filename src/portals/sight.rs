//! The one question the light-picking slices ask: may any of these cameras see part of a
//! sphere? Frustum first, then the rooms.

use bevy::camera::primitives::Frustum;
use bevy::math::Vec3;

use super::state::{RoomGraph, RoomView};
use crate::shared::sees_sphere;

/// Whether any of `cameras` may see part of the sphere at `center` with `radius`: it lies in
/// the camera's frustum (far plane ignored, as in [`sees_sphere`]) and, when the camera has a
/// bounded [`RoomView`] of the current graph, inside one of its visits to a node the sphere
/// reaches. The sphere must hold everything the light or volume can affect; walls are never
/// assumed to stop it, since realtime shadows can be missing or let light through.
pub fn sees<'a>(
    graph: &RoomGraph,
    cameras: impl IntoIterator<Item = (&'a Frustum, Option<&'a RoomView>)>,
    center: Vec3,
    radius: f32,
) -> bool {
    let ready = graph.ready();
    // The nodes asked about so far and whether the sphere reaches them, for every camera.
    let mut known: Vec<(usize, bool)> = Vec::new();
    for (frustum, view) in cameras {
        if !sees_sphere(frustum, center, radius) {
            continue;
        }
        let traced = view.and_then(|view| view.bounded(graph.generation()));
        let (Some(rooms), Some(traced)) = (ready, traced) else {
            return true;
        };
        let reaches = |node: usize| {
            if let Some(&(_, reached)) = known.iter().find(|&&(known, _)| known == node) {
                return reached;
            }
            let reached = rooms.reaches(node, center, radius);
            known.push((node, reached));
            reached
        };
        if traced.sees(reaches, center, radius, rooms.tolerance) {
            return true;
        }
    }
    false
}
