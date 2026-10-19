//! A camera's view through the portals. Starting in the nodes the eye and its near plane reach
//! ([`starts`]), with the camera's side planes and its near plane moved back to the eye, every
//! portal the view reaches narrows it to the planes through the eye and the portal's edges,
//! plus the portal's own plane. Each path to a node is one visit with its own planes, so two
//! doorways into one room stay two views of it.
//!
//! Every plane a visit keeps bounds every ray the eye can see along that path, so a sphere
//! outside one of them is out of sight there. A parent plane that holds the whole portal is
//! dropped: the side, near and edge planes pass through the eye and the eye lies behind every
//! portal plane, so such a plane holds everything past the portal too. Dropping a plane only
//! ever widens a view, as do the tolerances; anything the trace can't bound (too deep, too much
//! work) makes the whole view unbounded.

use bevy::math::{Vec3, Vec4, Vec4Swizzles};

use super::cell::distance;
use super::graph::Graph;
use super::near::starts;

/// Visits past this many make a view unbounded.
pub const VISIT_LIMIT: u32 = 4096;
/// Portal tests past this many make a view unbounded.
pub const PORTAL_TEST_LIMIT: u32 = 65_536;

/// The work a trace did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TraceCounts {
    pub visits: u32,
    pub portal_tests: u32,
    pub plane_tests: u32,
}

impl core::ops::AddAssign for TraceCounts {
    fn add_assign(&mut self, other: Self) {
        self.visits += other.visits;
        self.portal_tests += other.portal_tests;
        self.plane_tests += other.plane_tests;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Visit {
    node: usize,
    planes: (u32, u32),
}

/// What one camera sees: one visit per path to a node, each with the planes bounding what the
/// eye can see along it. Planes are relative to the eye.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TracedView {
    eye: Vec3,
    visits: Vec<Visit>,
    planes: Vec<Vec4>,
}

impl TracedView {
    /// Whether some visit to a node the sphere `reaches` may see part of it. The planes are
    /// tested first: `reaches` can cost a pass over every cell.
    pub fn sees(
        &self,
        mut reaches: impl FnMut(usize) -> bool,
        center: Vec3,
        radius: f32,
        tolerance: f32,
    ) -> bool {
        let center = center - self.eye;
        let reach_out = -(radius + tolerance);
        self.visits.iter().any(|visit| {
            self.planes(visit)
                .iter()
                .all(|&plane| distance(plane, center) >= reach_out)
                && reaches(visit.node)
        })
    }

    /// Whether the view reaches `node` at all.
    pub fn visits(&self, node: usize) -> bool {
        self.visits.iter().any(|visit| visit.node == node)
    }

    /// How many visits the view has.
    pub fn visit_count(&self) -> usize {
        self.visits.len()
    }

    /// The planes of every visit, relative to the eye, for tests.
    #[cfg(test)]
    pub fn visit_planes(&self) -> impl Iterator<Item = (usize, &[Vec4])> {
        self.visits
            .iter()
            .map(|visit| (visit.node, self.planes(visit)))
    }

    fn planes(&self, visit: &Visit) -> &[Vec4] {
        &self.planes[visit.planes.0 as usize..visit.planes.1 as usize]
    }
}

/// The camera a view is traced from.
#[derive(Clone, Copy, Debug)]
pub struct Viewer<'a> {
    pub eye: Vec3,
    /// The near plane's corners ([`near_corners`](super::near::near_corners)); empty for a
    /// point eye.
    pub near: &'a [Vec3],
    /// The camera's world planes in Bevy's order (four sides, near, far; the far plane is
    /// ignored).
    pub frustum: &'a [Vec4; 6],
}

/// Traces the view of `viewer` through `graph` into `view`. A portal whose entry in `closed` is
/// set lets nothing through; a shorter `closed` leaves the rest open. Returns `false` when the
/// view is unbounded: deeper than `depth_limit` portals, past the work limits, or from a viewer
/// that isn't finite.
pub fn trace(
    graph: &Graph,
    viewer: Viewer,
    closed: &[bool],
    depth_limit: u32,
    view: &mut TracedView,
    counts: &mut TraceCounts,
) -> bool {
    let Viewer { eye, near, frustum } = viewer;
    view.eye = eye;
    view.visits.clear();
    view.planes.clear();
    if !eye.is_finite()
        || !near.iter().all(|corner| corner.is_finite())
        || !frustum.iter().all(|plane| plane.is_finite())
    {
        return false;
    }
    // Relative to the eye; the side planes widened by the tolerance against cancellation, the
    // near plane moved back to the eye so a portal nearer than it still counts.
    let mut start: Vec<Vec4> = frustum[..4]
        .iter()
        .map(|&plane| plane.xyz().extend(distance(plane, eye) + graph.tolerance))
        .collect();
    start.push(frustum[4].xyz().extend(0.0));
    let mut homes = Vec::new();
    starts(graph, eye, near, &mut homes);
    let mut tracer = Tracer {
        graph,
        eye,
        closed,
        depth_limit,
        view,
        counts,
        path: Vec::new(),
    };
    homes.into_iter().all(|node| {
        tracer.path.clear();
        tracer.walk(node, start.clone(), 0, true)
    })
}

struct Tracer<'a> {
    graph: &'a Graph,
    eye: Vec3,
    closed: &'a [bool],
    depth_limit: u32,
    view: &'a mut TracedView,
    counts: &'a mut TraceCounts,
    path: Vec<usize>,
}

impl Tracer<'_> {
    /// Records the visit to `node` with `planes` and follows its portals; `false` once the view
    /// is unbounded.
    fn walk(&mut self, node: usize, planes: Vec<Vec4>, depth: u32, start: bool) -> bool {
        let first = self.view.planes.len() as u32;
        self.view.planes.extend_from_slice(&planes);
        let last = self.view.planes.len() as u32;
        self.view.visits.push(Visit {
            node,
            planes: (first, last),
        });
        self.counts.visits += 1;
        if self.counts.visits > VISIT_LIMIT {
            return false;
        }
        self.path.push(node);
        let tolerance = self.graph.tolerance;
        for link in self.graph.links(node) {
            if self.path.contains(&link.to) || self.closed.get(link.portal) == Some(&true) {
                continue;
            }
            self.counts.portal_tests += 1;
            if self.counts.portal_tests > PORTAL_TEST_LIMIT {
                return false;
            }
            let aperture = &self.graph.portals[link.portal].aperture;
            let center = aperture.center - self.eye;
            // How far the eye lies past the portal's plane, towards the node it leads to.
            let past = -link.outward.dot(center);
            let next = if past.abs() <= tolerance || (past > 0.0 && start) {
                // The eye stands in the doorway, or the start node holds it only loosely: see
                // through the portal with the planes unchanged.
                planes.clone()
            } else if past > 0.0 {
                // A ray crosses a plane once: nothing reaches through this portal from here.
                continue;
            } else {
                let corners = aperture.corners().map(|corner| corner - self.eye);
                match self.narrow(&planes, &corners, center, link.outward) {
                    Some(next) => next,
                    None => continue,
                }
            };
            if depth + 1 > self.depth_limit || !self.walk(link.to, next, depth + 1, false) {
                return false;
            }
        }
        self.path.pop();
        true
    }

    /// The planes past a portal with `corners` (relative to the eye), or `None` when `planes`
    /// hide it.
    fn narrow(
        &mut self,
        planes: &[Vec4],
        corners: &[Vec3; 4],
        center: Vec3,
        outward: Vec3,
    ) -> Option<Vec<Vec4>> {
        let tolerance = self.graph.tolerance;
        let mut next = Vec::with_capacity(planes.len() + 5);
        for &plane in planes {
            self.counts.plane_tests += 1;
            let distances = corners.map(|corner| distance(plane, corner));
            if distances.iter().all(|&d| d < -tolerance) {
                return None;
            }
            if distances.iter().any(|&d| d < 0.0) {
                next.push(plane);
            }
        }
        for (index, &a) in corners.iter().enumerate() {
            let b = corners[(index + 1) % 4];
            let normal = a.cross(b);
            let length = normal.length();
            // Seen edge-on, an edge's plane is unreliable; leaving it out only widens the view.
            if length <= 1.0e-6 * a.length() * b.length() {
                continue;
            }
            let normal = normal / length;
            let normal = if normal.dot(center) < 0.0 {
                -normal
            } else {
                normal
            };
            next.push(normal.extend(0.0));
        }
        next.push(outward.extend(-outward.dot(center)));
        Some(next)
    }
}
