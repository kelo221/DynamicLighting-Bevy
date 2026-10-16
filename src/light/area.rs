//! CPU reference for the area-light math in `shaders/area.wesl`, evaluated from the same
//! [`GpuLight`] record: rectangle panels and tubes, their nearest points, mean cosines (the
//! diffuse term) and representative points for specular light. Also the shape a baked area
//! light's soft shadows were traced for.

use bevy::math::{Quat, Vec2, Vec3};

use super::component::DynamicLight;
use super::kinds::LightType;
use crate::shared::{ChannelFlags, GpuLight};

/// Below this solid angle (or plane angle, for tubes) the shape counts as a point.
const MIN_ANGLE: f32 = 1.0e-6;
/// `1 − cos(θ / 2)` for the largest rotation θ (0.01 rad) that still matches a bake.
const SHAPE_ROTATION_TOLERANCE: f32 = 1.25e-5;
/// The largest size change in meters that still matches a bake.
const SHAPE_SIZE_TOLERANCE: f32 = 1.0e-3;

/// What an area light's baked soft shadows were traced for: its type, orientation and size. A
/// baked light whose shape no longer matches is lit as realtime, like a light that moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaLightShape {
    /// [`LightType::Rectangle`] or [`LightType::Tube`].
    pub kind: LightType,
    pub rotation: Quat,
    /// Width and height; a tube's height is 0.
    pub size: Vec2,
}

impl AreaLightShape {
    /// The shape of `light` turned by `rotation`; `None` unless it is a rectangle or tube.
    pub fn of(light: &DynamicLight, rotation: Quat) -> Option<Self> {
        light.light_type.is_area().then(|| Self {
            kind: light.light_type,
            rotation,
            size: light.half_extents() * 2.0,
        })
    }

    /// Half extents along the local X and Y axes.
    pub fn half_extents(&self) -> Vec2 {
        self.size * 0.5
    }

    /// The shape centered at `center`, with the axes [`super::pack`] gives the shaders.
    pub fn at(&self, center: Vec3) -> AreaShape {
        AreaShape {
            center,
            x: self.rotation * Vec3::X,
            y: self.rotation * Vec3::Y,
            forward: self.rotation * Vec3::NEG_Z,
            half: self.half_extents(),
            rectangle: self.kind == LightType::Rectangle,
        }
    }

    /// Whether a light shaped like `other` can still use shadows baked for this shape.
    pub fn matches(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.rotation.dot(other.rotation).abs() >= 1.0 - SHAPE_ROTATION_TOLERANCE
            && self.size.abs_diff_eq(other.size, SHAPE_SIZE_TOLERANCE)
    }
}

/// An area light's shape, from its record.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaShape {
    pub center: Vec3,
    /// Unit axes: the shape spans `±half.x` along `x` and `±half.y` along `y`.
    pub x: Vec3,
    pub y: Vec3,
    /// A rectangle lights only the side this points to.
    pub forward: Vec3,
    pub half: Vec2,
    pub rectangle: bool,
}

impl AreaShape {
    /// The shape of a rectangle or tube light; `None` for the other types.
    pub fn of(light: &GpuLight) -> Option<Self> {
        let kind = LightType::from_bits(ChannelFlags::unpack(light.channel).light_type)?;
        if !kind.is_area() {
            return None;
        }
        let forward = Vec3::from(light.forward);
        let up = Vec3::from(light.up);
        Some(Self {
            center: Vec3::from(light.position),
            x: forward.cross(up),
            y: up,
            forward,
            half: Vec2::new(light.gp1, light.gp2),
            rectangle: kind == LightType::Rectangle,
        })
    }

    /// The point of the shape nearest to `world`.
    pub fn nearest_point(&self, world: Vec3) -> Vec3 {
        let local = world - self.center;
        let x = local.dot(self.x).clamp(-self.half.x, self.half.x);
        let y = local.dot(self.y).clamp(-self.half.y, self.half.y);
        self.center + self.x * x + self.y * y
    }

    /// The point at `uv` in `[0, 1]²` across the shape; a tube only uses `uv.x`.
    pub fn point_at(&self, uv: Vec2) -> Vec3 {
        let offset = (uv * 2.0 - 1.0) * self.half;
        self.center + self.x * offset.x + self.y * offset.y
    }

    /// The corners of a rectangle, in order around it.
    pub fn corners(&self) -> [Vec3; 4] {
        let (hx, hy) = (self.x * self.half.x, self.y * self.half.y);
        let c = self.center;
        [c - hx - hy, c + hx - hy, c + hx + hy, c - hx + hy]
    }

    /// The ends of a tube.
    pub fn ends(&self) -> [Vec3; 2] {
        let hx = self.x * self.half.x;
        [self.center - hx, self.center + hx]
    }
}

fn safe_normalize(v: Vec3) -> Vec3 {
    v / v.length_squared().max(1.0e-20).sqrt()
}

/// The integral of the direction over the great arc between unit vectors `a` and `b`, times two.
fn edge(a: Vec3, b: Vec3) -> Vec3 {
    let c = a.cross(b);
    let s = c.length();
    c * (s.atan2(a.dot(b)) / s.max(1.0e-9))
}

fn triangle_solid_angle(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let triple = a.dot(b.cross(c)).abs();
    2.0 * triple.atan2(1.0 + a.dot(b) + b.dot(c) + c.dot(a))
}

/// The solid angle of the rectangle seen from `world`.
pub fn solid_angle(shape: &AreaShape, world: Vec3) -> f32 {
    let d = shape.corners().map(|corner| safe_normalize(corner - world));
    triangle_solid_angle(d[0], d[1], d[2]) + triangle_solid_angle(d[0], d[2], d[3])
}

/// Mean cosine between `normal` and the directions to the rectangle, over its solid angle; with
/// `clip`, only the part above the surface's horizon counts.
pub fn rectangle_cosine(shape: &AreaShape, world: Vec3, normal: Vec3, clip: bool) -> f32 {
    let corners = shape.corners().map(|corner| corner - world);
    let d = corners.map(safe_normalize);
    let solid = solid_angle(shape, world);
    if solid < MIN_ANGLE {
        return normal.dot(safe_normalize(shape.center - world));
    }
    if !clip {
        let vector = edge(d[0], d[1]) + edge(d[1], d[2]) + edge(d[2], d[3]) + edge(d[3], d[0]);
        let toward = if vector.dot(d[0] + d[1] + d[2] + d[3]) < 0.0 {
            -1.0
        } else {
            1.0
        };
        return 0.5 * toward * normal.dot(vector) / solid;
    }
    let mut clipped = Vec::with_capacity(5);
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        let (da, db) = (normal.dot(a), normal.dot(b));
        if da >= 0.0 {
            clipped.push(a);
        }
        if (da >= 0.0) != (db >= 0.0) {
            clipped.push(a.lerp(b, da / (da - db)));
        }
    }
    if clipped.len() < 3 {
        return 0.0;
    }
    let count = clipped.len();
    let vector: Vec3 = (0..count)
        .map(|i| {
            edge(
                safe_normalize(clipped[i]),
                safe_normalize(clipped[(i + 1) % count]),
            )
        })
        .sum();
    0.5 * normal.dot(vector).abs() / solid
}

/// The integral of the direction over the arc from unit vector `a` to unit vector `b`.
fn arc_vector(a: Vec3, b: Vec3) -> Vec3 {
    (a + b) * (a.cross(b).length() / (1.0 + a.dot(b)).max(1.0e-6))
}

/// Mean cosine between `normal` and the directions to the tube, over the angle it covers; with
/// `clip`, only the part above the surface's horizon counts.
pub fn tube_cosine(shape: &AreaShape, world: Vec3, normal: Vec3, clip: bool) -> f32 {
    let [p0, p1] = shape.ends().map(|end| end - world);
    let (a, b) = (safe_normalize(p0), safe_normalize(p1));
    let angle = a.cross(b).length().atan2(a.dot(b));
    if angle < MIN_ANGLE {
        return normal.dot(safe_normalize(shape.center - world));
    }
    let (mut p0, mut p1) = (p0, p1);
    if clip {
        let (d0, d1) = (normal.dot(p0), normal.dot(p1));
        if d0 <= 0.0 && d1 <= 0.0 {
            return 0.0;
        }
        let crossing = p0.lerp(p1, d0 / (d0 - d1));
        if d0 < 0.0 {
            p0 = crossing;
        } else if d1 < 0.0 {
            p1 = crossing;
        }
    }
    normal.dot(arc_vector(safe_normalize(p0), safe_normalize(p1))) / angle
}

/// The diffuse term of an area light at a surface: the mean cosine, clipped to the horizon with
/// plain Lambert on `baked` surfaces and wrapped with Henry's half-Lambert elsewhere, times a
/// rectangle's emission cosine.
pub fn diffuse(shape: &AreaShape, world: Vec3, normal: Vec3, baked: bool) -> f32 {
    let mut emission = 1.0;
    let cosine = if shape.rectangle {
        emission = shape
            .forward
            .dot(safe_normalize(world - shape.nearest_point(world)));
        if emission <= 0.0 {
            return 0.0;
        }
        rectangle_cosine(shape, world, normal, baked)
    } else {
        tube_cosine(shape, world, normal, baked)
    };
    if baked {
        return cosine.max(0.0) * emission;
    }
    let half_lambert = 0.5 + 0.5 * cosine;
    half_lambert * half_lambert * emission
}

/// The largest cosine between `normal` and a direction on the great arc from unit vector `a` to
/// unit vector `b`, shorter than a half turn.
fn arc_facing(a: Vec3, b: Vec3, normal: Vec3) -> f32 {
    let mut best = normal.dot(a).max(normal.dot(b));
    let c = a.cross(b);
    let s = c.length();
    if s > MIN_ANGLE {
        let axis = c / s;
        let in_plane = normal - axis * normal.dot(axis);
        if a.cross(in_plane).dot(axis) > 0.0 && in_plane.cross(b).dot(axis) > 0.0 {
            best = best.max(in_plane.length());
        }
    }
    best
}

/// The cosine between `normal` and the direction to the point of the shape the surface faces
/// most. Above 0 exactly when part of the shape is above the surface's horizon; a point light's
/// cosine as the shape shrinks. Realtime shadows fade their terminator with it.
pub fn facing_cosine(shape: &AreaShape, world: Vec3, normal: Vec3) -> f32 {
    if !shape.rectangle {
        let [a, b] = shape.ends().map(|end| safe_normalize(end - world));
        return arc_facing(a, b, normal);
    }
    let base = shape.center - world;
    let denominator = normal.dot(shape.forward);
    if denominator.abs() > 1.0e-6 {
        let t = base.dot(shape.forward) / denominator;
        let hit = normal * t - base;
        if t > 0.0
            && hit.dot(shape.x).abs() <= shape.half.x
            && hit.dot(shape.y).abs() <= shape.half.y
        {
            return 1.0;
        }
    }
    let d = shape.corners().map(|corner| safe_normalize(corner - world));
    (0..4)
        .map(|i| arc_facing(d[i], d[(i + 1) % 4], normal))
        .fold(-1.0, f32::max)
}

/// The point of the shape whose reflection a surface at `world` shows along `reflection`.
pub fn representative_point(shape: &AreaShape, world: Vec3, reflection: Vec3) -> Vec3 {
    if !shape.rectangle {
        let [start, end] = shape.ends();
        let (l0, ld) = (start - world, end - start);
        let r_ld = reflection.dot(ld);
        let t = (reflection.dot(l0) * r_ld - l0.dot(ld)) / (ld.dot(ld) - r_ld * r_ld).max(1.0e-6);
        return start + ld * t.clamp(0.0, 1.0);
    }
    let mut aim = world + reflection * (shape.center - world).length();
    let denominator = reflection.dot(shape.forward);
    if denominator.abs() > 1.0e-4 {
        let t = (shape.center - world).dot(shape.forward) / denominator;
        if t > 0.0 {
            aim = world + reflection * t;
        }
    }
    shape.nearest_point(aim)
}

/// Karis's energy normalization for a representative `point`, per axis of the shape.
pub fn specular_scale(shape: &AreaShape, world: Vec3, point: Vec3, roughness: f32) -> f32 {
    if shape.rectangle && shape.forward.dot(world - point) <= 0.0 {
        return 0.0;
    }
    let alpha = roughness * roughness;
    let spread = shape.half / (3.0 * shape.center.distance(world).max(1.0e-4));
    let widened = (Vec2::splat(alpha) + spread).clamp(Vec2::ZERO, Vec2::ONE);
    let scale = alpha / widened.max(Vec2::splat(1.0e-6));
    scale.x * if shape.rectangle { scale.y } else { 1.0 }
}
