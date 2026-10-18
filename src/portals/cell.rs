//! Geometry of rooms and portals: room cells (a box under any non-singular affine map, so a
//! parent's shear is exact), portal apertures (a parallelogram), and the tests the graph and the
//! trace share. Planes are `Vec4(normal, d)` with a unit normal, positive inside.

use bevy::math::{Affine3A, Vec2, Vec3, Vec4, Vec4Swizzles};

/// Signed distance of `point` from `plane`, positive inside.
pub fn distance(plane: Vec4, point: Vec3) -> f32 {
    plane.xyz().dot(point) + plane.w
}

/// The points `center + Σ tᵢ · axes[i]` with every `|tᵢ| ≤ 1`: a box under an affine map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parallelepiped {
    pub center: Vec3,
    /// Half edges: from the center to the middle of each pair of faces.
    pub axes: [Vec3; 3],
}

impl Parallelepiped {
    /// The box `[-half, half]` under `affine`, or `None` when that is flat, singular or not
    /// finite.
    pub fn from_affine(affine: Affine3A, half: Vec3) -> Option<Self> {
        let matrix = affine.matrix3;
        let shape = Self {
            center: affine.translation.into(),
            axes: [
                Vec3::from(matrix.x_axis) * half.x,
                Vec3::from(matrix.y_axis) * half.y,
                Vec3::from(matrix.z_axis) * half.z,
            ],
        };
        shape.is_solid().then_some(shape)
    }

    /// Finite, with every edge and the volume clearly above zero.
    fn is_solid(&self) -> bool {
        let [a, b, c] = self.axes;
        let lengths = a.length() * b.length() * c.length();
        self.center.is_finite()
            && self.axes.iter().all(|axis| axis.is_finite())
            && lengths.is_finite()
            && lengths > 1.0e-12
            && a.cross(b).dot(c).abs() > 1.0e-4 * lengths
    }

    /// Unit face normals, each pointing out along its axis.
    pub fn face_normals(&self) -> [Vec3; 3] {
        let [a, b, c] = self.axes;
        let normals = [b.cross(c), c.cross(a), a.cross(b)];
        core::array::from_fn(|index| {
            let normal = normals[index].normalize_or_zero();
            if normal.dot(self.axes[index]) < 0.0 {
                -normal
            } else {
                normal
            }
        })
    }

    /// The six face planes, positive inside.
    pub fn planes(&self) -> [Vec4; 6] {
        let normals = self.face_normals();
        core::array::from_fn(|index| {
            let normal = normals[index / 2];
            let reach = normal.dot(self.axes[index / 2]);
            let at = normal.dot(self.center);
            if index % 2 == 0 {
                (-normal).extend(at + reach)
            } else {
                normal.extend(reach - at)
            }
        })
    }

    /// The eight corners.
    pub fn corners(&self) -> [Vec3; 8] {
        let [a, b, c] = self.axes;
        core::array::from_fn(|index| {
            let sign = |bit: usize| if index & bit == 0 { -1.0 } else { 1.0 };
            self.center + a * sign(1) + b * sign(2) + c * sign(4)
        })
    }

    /// Half the extent of the projection onto the unit `axis`.
    fn reach(&self, axis: Vec3) -> f32 {
        self.axes.iter().map(|edge| edge.dot(axis).abs()).sum()
    }

    /// The world-space bounds, as (min, max).
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let reach = self.axes.iter().map(|edge| edge.abs()).sum::<Vec3>();
        (self.center - reach, self.center + reach)
    }

    /// How deeply this and `other` overlap: the smallest overlap of their projections over the
    /// separating axes (face normals of both, then cross products of their edges). Zero or less
    /// when they are apart.
    pub fn overlap(&self, other: &Self) -> f32 {
        let (low, high) = self.bounds();
        let (other_low, other_high) = other.bounds();
        let apart = (low - other_high).max(other_low - high).max_element();
        if apart > 0.0 {
            return -apart;
        }
        let mut depth = f32::INFINITY;
        let mut test = |axis: Vec3| {
            if let Some(axis) = axis.try_normalize() {
                let gap = (other.center - self.center).dot(axis).abs();
                depth = depth.min(self.reach(axis) + other.reach(axis) - gap);
            }
        };
        self.face_normals().into_iter().for_each(&mut test);
        other.face_normals().into_iter().for_each(&mut test);
        for edge in self.axes {
            for other_edge in other.axes {
                test(edge.cross(other_edge));
            }
        }
        depth
    }
}

/// A room's cell: its shape and face planes.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub shape: Parallelepiped,
    pub planes: [Vec4; 6],
}

impl Cell {
    pub fn new(shape: Parallelepiped) -> Self {
        Self {
            shape,
            planes: shape.planes(),
        }
    }

    /// The smallest signed distance of `point` from the faces: positive inside.
    pub fn depth(&self, point: Vec3) -> f32 {
        self.planes
            .iter()
            .map(|&plane| distance(plane, point))
            .fold(f32::INFINITY, f32::min)
    }

    /// Whether the sphere may reach into the cell. Exact against each face; near edges and
    /// corners it can say yes for a sphere that just misses.
    pub fn touches_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.depth(center) >= -radius
    }

    /// Whether the whole sphere lies inside the cell.
    pub fn holds_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.depth(center) >= radius
    }
}

/// A portal's opening: the parallelogram `center ± u ± v`, seen through along `normal`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aperture {
    pub center: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    /// Unit normal, `u × v` normalized: the front side.
    pub normal: Vec3,
}

impl Aperture {
    /// The rectangle `size` in the local XY plane under `affine`, or `None` when that is flat
    /// or not finite.
    pub fn from_affine(affine: Affine3A, size: Vec2) -> Option<Self> {
        let matrix = affine.matrix3;
        let u = Vec3::from(matrix.x_axis) * size.x * 0.5;
        let v = Vec3::from(matrix.y_axis) * size.y * 0.5;
        let center = Vec3::from(affine.translation);
        let area = u.cross(v);
        let lengths = u.length() * v.length();
        let solid = center.is_finite()
            && area.is_finite()
            && lengths > 1.0e-12
            && area.length() > 1.0e-4 * lengths;
        solid.then(|| Self {
            center,
            u,
            v,
            normal: area.normalize(),
        })
    }

    /// The corners, in order around the edge.
    pub fn corners(&self) -> [Vec3; 4] {
        let (c, u, v) = (self.center, self.u, self.v);
        [c - u - v, c + u - v, c + u + v, c - u + v]
    }

    /// The corners pulled towards the center by up to `inset` along each edge, keeping at
    /// least half the opening.
    pub fn inset_corners(&self, inset: f32) -> [Vec3; 4] {
        let pull = |edge: Vec3| {
            let length = edge.length();
            edge * (1.0 - (inset / length.max(1.0e-12)).min(0.5))
        };
        let (c, u, v) = (self.center, pull(self.u), pull(self.v));
        [c - u - v, c + u - v, c + u + v, c - u + v]
    }

    /// The opening swept along `side` (+1 front, −1 back) from `from` to `to` meters off its
    /// plane.
    pub fn swept(&self, side: f32, from: f32, to: f32) -> Parallelepiped {
        let along = self.normal * side;
        Parallelepiped {
            center: self.center + along * (from + to) * 0.5,
            axes: [self.u, self.v, along * (to - from) * 0.5],
        }
    }
}

/// Where the ray `origin + direction · t` lies inside every plane, as `(low, high)` over all
/// `t`, or `None` when it misses.
pub fn ray_span(planes: &[Vec4], origin: Vec3, direction: Vec3) -> Option<(f32, f32)> {
    let (mut low, mut high) = (f32::NEG_INFINITY, f32::INFINITY);
    for &plane in planes {
        let start = distance(plane, origin);
        let rate = plane.xyz().dot(direction);
        if rate.abs() < 1.0e-9 {
            if start < 0.0 {
                return None;
            }
        } else if rate > 0.0 {
            low = low.max(-start / rate);
        } else {
            high = high.min(-start / rate);
        }
    }
    (low <= high).then_some((low, high))
}
