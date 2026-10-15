//! Small geometry helpers for the tracer: boxes, triangle areas, sphere-triangle tests and UV
//! barycentrics.

use bevy::math::{Vec2, Vec3};

/// An axis-aligned box. Overlap tests include touching faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    pub const EMPTY: Self = Self {
        min: Vec3::INFINITY,
        max: Vec3::NEG_INFINITY,
    };

    pub fn from_points(points: &[Vec3]) -> Self {
        points.iter().fold(Self::EMPTY, |aabb, &p| aabb.grow(p))
    }

    pub fn from_sphere(center: Vec3, radius: f32) -> Self {
        Self {
            min: center - Vec3::splat(radius),
            max: center + Vec3::splat(radius),
        }
    }

    pub fn grow(self, point: Vec3) -> Self {
        Self {
            min: self.min.min(point),
            max: self.max.max(point),
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn intersects(&self, other: &Self) -> bool {
        self.min.cmple(other.max).all() && other.min.cmple(self.max).all()
    }

    /// Twice the surface area; empty boxes give 0.
    pub fn half_area_x2(&self) -> f32 {
        let e = self.max - self.min;
        if e.min_element() < 0.0 {
            return 0.0;
        }
        2.0 * (e.x * e.y + e.y * e.z + e.z * e.x)
    }
}

/// Area of a 3D triangle.
pub fn triangle_area(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    (b - a).cross(c - a).length() * 0.5
}

/// Side of a mesh's square shadow map: `ceil(sqrt(density² · area))` (Henry's
/// `SurfaceAreaToTextureSize`), clamped to `[2, max_size]`. The lower bound avoids Henry's
/// size-1 maps, whose texel math reads out of bounds.
pub fn lightmap_size(surface_area: f32, pixel_density: f32, max_size: u32) -> u32 {
    let size = (pixel_density * pixel_density * surface_area).sqrt().ceil();
    let size = if size.is_nan() { 0.0 } else { size };
    (size.min(max_size as f32) as u32).clamp(2, max_size.max(2))
}

/// The point of triangle `abc` closest to `p` (Ericson, "Real-Time Collision Detection").
pub fn closest_point_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// Whether a sphere touches triangle `abc`.
pub fn sphere_intersects_triangle(center: Vec3, radius: f32, a: Vec3, b: Vec3, c: Vec3) -> bool {
    let closest = closest_point_on_triangle(center, a, b, c);
    closest.distance_squared(center) <= radius * radius
}

/// Henry's signed 2D triangle area ("2D cross product").
pub fn signed_area(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    let a = a - c;
    let b = b - c;
    (a.x * b.y - a.y * b.x) * 0.5
}

/// Barycentric weights of `point` in UV triangle `t`, or `None` when it lies outside (any
/// weight negative) or the triangle has no UV area.
pub fn uv_barycentrics(t: [Vec2; 3], point: Vec2) -> Option<Vec3> {
    let area = signed_area(t[0], t[1], t[2]);
    if area == 0.0 {
        return None;
    }
    let a1 = signed_area(t[1], t[2], point) / area;
    let a2 = signed_area(t[2], t[0], point) / area;
    let a3 = signed_area(t[0], t[1], point) / area;
    (a1 >= 0.0 && a2 >= 0.0 && a3 >= 0.0).then_some(Vec3::new(a1, a2, a3))
}

/// Component-wise `|a - b| < epsilon`. Henry's `ApproximatelyEquals` compares absolute values,
/// so it calls `(0, 1, 0)` and `(0, -1, 0)` equal; this doesn't.
pub fn approx_eq(a: Vec3, b: Vec3, epsilon: f32) -> bool {
    (a - b).abs().max_element() < epsilon
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightmap_size_follows_density_and_clamps() {
        assert_eq!(lightmap_size(1.0, 128.0, 2048), 128);
        assert_eq!(lightmap_size(4.0, 128.0, 2048), 256);
        assert_eq!(lightmap_size(2.0, 10.0, 2048), 15); // ceil(14.14)
        assert_eq!(lightmap_size(1e6, 128.0, 2048), 2048);
        assert_eq!(lightmap_size(0.0, 128.0, 2048), 2);
        assert_eq!(lightmap_size(1e-9, 128.0, 2048), 2);
        assert_eq!(lightmap_size(f32::NAN, 128.0, 2048), 2);
        assert_eq!(lightmap_size(f32::INFINITY, 128.0, 64), 64);
        assert_eq!(lightmap_size(1.0, 128.0, 1), 2);
    }

    #[test]
    fn closest_point_covers_every_region() {
        let (a, b, c) = (Vec3::ZERO, Vec3::X, Vec3::Y);
        let cases = [
            (Vec3::new(0.25, 0.25, 1.0), Vec3::new(0.25, 0.25, 0.0)),
            (Vec3::new(-1.0, -1.0, 0.0), a),
            (Vec3::new(2.0, -0.5, 0.0), b),
            (Vec3::new(-0.5, 2.0, 0.0), c),
            (Vec3::new(0.5, -1.0, 0.0), Vec3::new(0.5, 0.0, 0.0)),
            (Vec3::new(-1.0, 0.5, 0.0), Vec3::new(0.0, 0.5, 0.0)),
            (Vec3::new(1.0, 1.0, 0.0), Vec3::new(0.5, 0.5, 0.0)),
        ];
        for (p, expected) in cases {
            let found = closest_point_on_triangle(p, a, b, c);
            assert!(found.abs_diff_eq(expected, 1e-6), "{p}: {found}");
        }
        assert!(sphere_intersects_triangle(
            Vec3::new(0.2, 0.2, 0.5),
            0.5,
            a,
            b,
            c
        ));
        assert!(!sphere_intersects_triangle(
            Vec3::new(0.2, 0.2, 0.6),
            0.5,
            a,
            b,
            c
        ));
    }

    #[test]
    fn barycentrics_detect_outside_points_and_both_windings() {
        let t = [Vec2::ZERO, Vec2::X, Vec2::Y];
        let w = uv_barycentrics(t, Vec2::new(0.25, 0.25)).unwrap();
        assert!(w.abs_diff_eq(Vec3::new(0.5, 0.25, 0.25), 1e-6));
        assert!(uv_barycentrics(t, Vec2::new(0.75, 0.75)).is_none());
        let flipped = [Vec2::ZERO, Vec2::Y, Vec2::X];
        let w = uv_barycentrics(flipped, Vec2::new(0.25, 0.25)).unwrap();
        assert!(w.abs_diff_eq(Vec3::new(0.5, 0.25, 0.25), 1e-6));
        assert!(uv_barycentrics([Vec2::ZERO, Vec2::X, Vec2::X * 2.0], Vec2::X).is_none());
    }

    #[test]
    fn boxes_and_areas() {
        let a = Aabb::from_points(&[Vec3::ZERO, Vec3::ONE]);
        let b = Aabb::from_sphere(Vec3::splat(2.0), 1.0);
        assert!(a.intersects(&b), "touching boxes overlap");
        assert!(!a.intersects(&Aabb::from_sphere(Vec3::splat(2.0), 0.9)));
        assert_eq!(a.half_area_x2(), 6.0);
        assert_eq!(Aabb::EMPTY.half_area_x2(), 0.0);
        assert_eq!(triangle_area(Vec3::ZERO, Vec3::X * 2.0, Vec3::Y * 2.0), 2.0);
        assert!(approx_eq(Vec3::Y, Vec3::Y * 1.000001, 1e-5));
        assert!(!approx_eq(Vec3::Y, -Vec3::Y, 1e-5));
    }
}
