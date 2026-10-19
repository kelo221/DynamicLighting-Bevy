//! Where a view starts. The renderer clips everything nearer than the camera's near plane, walls
//! and closed doors included, so a camera close to a thin wall sees past it. A view therefore
//! starts in every node its near pyramid (the eye and the near plane's corners) reaches, not only
//! in the nodes that hold the eye.

use bevy::math::{Affine3A, Mat3, Mat4, Vec3, Vec4, Vec4Swizzles};

use super::cell::{Cell, distance};
use super::graph::{CONTAIN_EPSILON, Graph, OUTSIDE};

/// Corners within this of every plane count as corners of the polytope.
const VERTEX_SLACK: f32 = 1.0e-5;
/// Meeting points of three planes the containment test may try before it gives up and lets the
/// view start outside too.
const VERTEX_WORK: u32 = 20_000;

/// The corners of the near plane in world space, in order around the edge, for a camera with
/// `clip_from_view` (Bevy's reverse-Z: the near plane is at depth 1) placed at `world_from_view`.
/// `None` for a projection that can't be inverted.
pub fn near_corners(clip_from_view: Mat4, world_from_view: Affine3A) -> Option<[Vec3; 4]> {
    let view_from_clip = clip_from_view.inverse();
    let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| {
        let view = view_from_clip.project_point3(Vec3::new(x, y, 1.0));
        world_from_view.transform_point3(view)
    });
    corners
        .iter()
        .all(|corner| corner.is_finite())
        .then_some(corners)
}

/// The nodes a view from `eye` with the near-plane corners `near` starts in, into `out`: the
/// nodes that hold the eye ([`Graph::homes`]), every cell the near pyramid may touch, and the
/// outside unless the pyramid lies inside those cells. Without four corners, or with a flat
/// pyramid, the eye's nodes alone.
pub fn starts(graph: &Graph, eye: Vec3, near: &[Vec3], out: &mut Vec<usize>) {
    graph.homes(eye, out);
    let Some(pyramid) = <[Vec3; 4]>::try_from(near)
        .ok()
        .and_then(|near| Pyramid::new(eye, near))
    else {
        return;
    };
    let mut touched = Vec::new();
    for (index, cell) in graph.cells.iter().enumerate() {
        if pyramid.touches(cell, graph.tolerance) {
            touched.push(cell);
            if !out.contains(&(index + 1)) {
                out.push(index + 1);
            }
        }
    }
    if !out.contains(&OUTSIDE) && !pyramid.inside(&touched) {
        out.push(OUTSIDE);
    }
}

/// The near pyramid, relative to the eye so the containment test keeps its precision far from
/// the origin.
struct Pyramid {
    eye: Vec3,
    /// The eye and the four corners, relative to the eye.
    points: [Vec3; 5],
    /// The five faces, positive inside, relative to the eye.
    planes: [Vec4; 5],
    center: Vec3,
    radius: f32,
}

impl Pyramid {
    fn new(eye: Vec3, near: [Vec3; 4]) -> Option<Self> {
        let [a, b, c, d] = near.map(|corner| corner - eye);
        let points = [Vec3::ZERO, a, b, c, d];
        let center = points.iter().sum::<Vec3>() / 5.0;
        let radius = points
            .iter()
            .map(|point| point.distance(center))
            .fold(0.0, f32::max);
        if !eye.is_finite() || !radius.is_finite() || radius < 1.0e-6 {
            return None;
        }
        let faces = [
            [Vec3::ZERO, a, b],
            [Vec3::ZERO, b, c],
            [Vec3::ZERO, c, d],
            [Vec3::ZERO, d, a],
            [a, b, c],
        ];
        let mut planes = [Vec4::ZERO; 5];
        for (plane, [p, q, r]) in planes.iter_mut().zip(faces) {
            let normal = (q - p).cross(r - p).try_normalize()?;
            let normal = if normal.dot(center - p) < 0.0 {
                -normal
            } else {
                normal
            };
            *plane = normal.extend(-normal.dot(p));
        }
        Some(Self {
            eye,
            points,
            planes,
            center: center + eye,
            radius,
        })
    }

    /// The cell's face planes relative to the eye.
    fn cell_planes(&self, cell: &Cell) -> [Vec4; 6] {
        cell.planes
            .map(|plane| plane.xyz().extend(plane.w + plane.xyz().dot(self.eye)))
    }

    /// Whether the pyramid may touch `cell`: no face of either has the other wholly more than
    /// `slack` outside it. Without the edge axes it can say yes for a cell that just misses.
    fn touches(&self, cell: &Cell, slack: f32) -> bool {
        if !cell.touches_sphere(self.center, self.radius + slack) {
            return false;
        }
        let apart =
            |plane: Vec4, points: &[Vec3]| points.iter().all(|&p| distance(plane, p) < -slack);
        if self
            .cell_planes(cell)
            .iter()
            .any(|&plane| apart(plane, &self.points))
        {
            return false;
        }
        let corners = cell.shape.corners().map(|corner| corner - self.eye);
        !self.planes.iter().any(|&plane| apart(plane, &corners))
    }

    /// Whether the pyramid lies inside the union of `cells`, to within the containment epsilon.
    /// Gives `false` when the test would take too long.
    fn inside(&self, cells: &[&Cell]) -> bool {
        let cells: Vec<[Vec4; 6]> = cells.iter().map(|cell| self.cell_planes(cell)).collect();
        let mut work = 0;
        covered(self.planes.to_vec(), &cells, &mut work) && work <= VERTEX_WORK
    }
}

/// Whether the polytope inside every plane of `piece` lies inside the union of the cells. The
/// part outside the first cell is cut into one convex piece per face, each of which must lie in
/// the remaining cells.
fn covered(piece: Vec<Vec4>, cells: &[[Vec4; 6]], work: &mut u32) -> bool {
    let mut corners = vertices(&piece, work);
    if corners.is_empty() {
        return true;
    }
    let Some((cell, rest)) = cells.split_first() else {
        return false;
    };
    let within = |plane: Vec4, corners: &[Vec3]| {
        corners
            .iter()
            .all(|&corner| distance(plane, corner) >= -CONTAIN_EPSILON)
    };
    let mut inside = piece;
    for &plane in cell {
        // A face with the whole remaining piece inside it cuts nothing off.
        if within(plane, &corners) {
            continue;
        }
        if *work > VERTEX_WORK {
            return false;
        }
        let mut outside = inside.clone();
        outside.push((-plane.xyz()).extend(-plane.w - CONTAIN_EPSILON));
        if !covered(outside, rest, work) {
            return false;
        }
        inside.push(plane.xyz().extend(plane.w + CONTAIN_EPSILON));
        corners = vertices(&inside, work);
        if corners.is_empty() {
            break;
        }
    }
    true
}

/// The corners of the bounded polytope inside every plane: the meeting points of three planes
/// that lie inside the rest. Empty when the polytope is.
fn vertices(planes: &[Vec4], work: &mut u32) -> Vec<Vec3> {
    let mut corners = Vec::new();
    for a in 0..planes.len() {
        for b in a + 1..planes.len() {
            for c in b + 1..planes.len() {
                *work += 1;
                let rows =
                    Mat3::from_cols(planes[a].xyz(), planes[b].xyz(), planes[c].xyz()).transpose();
                if rows.determinant().abs() < 1.0e-9 {
                    continue;
                }
                let point = rows.inverse() * -Vec3::new(planes[a].w, planes[b].w, planes[c].w);
                if planes
                    .iter()
                    .all(|&plane| distance(plane, point) >= -VERTEX_SLACK)
                {
                    corners.push(point);
                }
            }
        }
    }
    corners
}
