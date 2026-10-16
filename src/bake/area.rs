//! Soft shadows of rectangle and tube lights: per texel, shadow rays to points spread over the
//! shape, each weighted by how much light that part of the shape sends to the texel. Distance
//! cubes and bounce light still come from the light's center.

use bevy::math::{Vec2, Vec3};

use crate::effects::EffectRng;
use crate::light::area::AreaShape;

use super::input::BakeLight;
use super::mesh_bake::NDOTL_TOLERANCE;

/// Shadow bits per texel of area lights: 16 levels of visibility.
pub const AREA_SHADOW_BITS: u32 = 4;
/// Shape points probed before deciding whether a texel lies in a penumbra: a rectangle's corners
/// and center, a tube's ends and middle. The part of a shape an occluder hides is convex, and in
/// a penumbra it almost always covers a corner, an end or the middle.
const RECTANGLE_PROBES: [Vec2; 5] = [
    Vec2::new(0.0, 0.0),
    Vec2::new(1.0, 0.0),
    Vec2::new(1.0, 1.0),
    Vec2::new(0.0, 1.0),
    Vec2::new(0.5, 0.5),
];
const TUBE_PROBES: [Vec2; 3] = [
    Vec2::new(0.0, 0.5),
    Vec2::new(1.0, 0.5),
    Vec2::new(0.5, 0.5),
];
/// Shadow rays stop this far short of the shape, so a light mounted flush on a surface doesn't
/// shadow itself.
const END_GAP: f32 = 1.0e-3;
/// Steps of the sample sequences: Roberts' R2 sequence (`1/g`, `1/g²` with g the plastic
/// number) over rectangles, the golden ratio along tubes. Both spread any number of points
/// evenly over the shape.
const R2_STEP: Vec2 = Vec2::new(0.754_877_7, 0.569_840_3);
const GOLDEN_STEP: f32 = 0.618_034;

impl BakeLight {
    /// The world-space shape of an area light.
    pub fn area_shape(&self) -> Option<AreaShape> {
        self.area.map(|area| area.at(self.position))
    }

    /// How far the light reaches past its shape: the radius minus half the shape's diagonal, as
    /// at runtime, so every lit texel lies within `radius` of the position. The radius itself
    /// for point-like lights.
    pub fn shape_reach(&self) -> f32 {
        self.radius - self.area.map_or(0.0, |area| area.half_extents().length())
    }

    /// Bits per texel of the light's baked shadows.
    pub fn shadow_bits_per_texel(&self) -> u32 {
        if self.area.is_some() {
            AREA_SHADOW_BITS
        } else {
            1
        }
    }

    /// Henry's facing test for a flat triangle with unit face `normal`: whether the light may be
    /// in front of it, seen from its center. An area light passes when any corner of its shape
    /// does; a rectangle also needs a vertex of the triangle on its lit side.
    pub fn may_face(&self, vertices: [Vec3; 3], normal: Vec3) -> bool {
        let center = (vertices[0] + vertices[1] + vertices[2]) / 3.0;
        let faces =
            |point: Vec3| normal.dot((point - center).normalize_or_zero()) > NDOTL_TOLERANCE;
        let Some(shape) = self.area_shape() else {
            return faces(self.position);
        };
        let lit_side = !shape.rectangle
            || vertices
                .iter()
                .any(|&vertex| shape.forward.dot(vertex - shape.center) > 0.0);
        lit_side && shape.corners().into_iter().any(faces)
    }
}

/// What one texel sees of an area light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TexelVisibility {
    /// The share of the light reaching the texel unblocked, 0 to 1.
    pub fraction: f32,
    pub rays: u32,
}

/// How much of `shape` reaches a texel at `position` with unit `normal`.
///
/// Shadow rays first go to the probe points; when they all agree the texel is fully lit or fully
/// shadowed. Otherwise `samples` rays go to points spread over the shape, the sequence shifted by
/// `rng`, each weighted by the light that part of the shape sends: receiver cosine (with Henry's
/// facing tolerance) × emitter cosine / distance². `None` when the texel is farther than `reach`
/// from the shape (or `reach` isn't positive), the shape is below its horizon, or it is behind a
/// rectangle.
/// `occluded(direction, distance)` traces one ray from the texel.
pub fn texel_visibility(
    shape: &AreaShape,
    reach: f32,
    (position, normal): (Vec3, Vec3),
    samples: u32,
    rng: &mut EffectRng,
    mut occluded: impl FnMut(Vec3, f32) -> bool,
) -> Option<TexelVisibility> {
    if reach <= 0.0 || position.distance_squared(shape.nearest_point(position)) > reach * reach {
        return None;
    }
    if shape.rectangle && shape.forward.dot(position - shape.center) <= 0.0 {
        return None;
    }
    let mut rays = 0;
    let mut trace = |uv: Vec2| {
        let (direction, distance, weight) = toward(shape, position, normal, uv)?;
        rays += 1;
        Some((weight, !occluded(direction, (distance - END_GAP).max(0.0))))
    };

    let (probes, step) = if shape.rectangle {
        (&RECTANGLE_PROBES[..], R2_STEP)
    } else {
        (&TUBE_PROBES[..], Vec2::new(GOLDEN_STEP, 0.0))
    };
    let (mut any_lit, mut any_blocked) = (false, false);
    for (_, lit) in probes.iter().filter_map(|&uv| trace(uv)) {
        any_lit |= lit;
        any_blocked |= !lit;
    }
    let mut fraction = (any_lit != any_blocked).then_some(if any_lit { 1.0 } else { 0.0 });
    if fraction.is_none() {
        let mut uv = Vec2::new(rng.next_f32(), rng.next_f32());
        let (mut lit, mut total) = (0.0, 0.0);
        for _ in 0..samples {
            if let Some((weight, unblocked)) = trace(uv) {
                total += weight;
                lit += if unblocked { weight } else { 0.0 };
            }
            uv = (uv + step).fract();
        }
        fraction = (total > 0.0).then(|| lit / total);
    }
    fraction.map(|fraction| TexelVisibility { fraction, rays })
}

/// The direction, distance and weight of the shape's point at `uv` seen from a texel, or `None`
/// when it sends no light there.
fn toward(shape: &AreaShape, position: Vec3, normal: Vec3, uv: Vec2) -> Option<(Vec3, f32, f32)> {
    let to_point = shape.point_at(uv) - position;
    let distance_sqr = to_point.length_squared();
    if distance_sqr < 1.0e-12 {
        return None;
    }
    let distance = distance_sqr.sqrt();
    let direction = to_point / distance;
    let receiver = normal.dot(direction) - NDOTL_TOLERANCE;
    let emitter = if shape.rectangle {
        -shape.forward.dot(direction)
    } else {
        1.0
    };
    (receiver > 0.0 && emitter > 0.0)
        .then(|| (direction, distance, receiver * emitter / distance_sqr))
}

#[cfg(test)]
mod tests {
    use bevy::math::Quat;

    use super::*;
    use crate::bake::geometry::Aabb;
    use crate::bake::random;
    use crate::light::{AreaLightShape, LightType};

    /// A 2 × 1 panel at the origin facing down.
    fn panel() -> AreaShape {
        AreaLightShape {
            kind: LightType::Rectangle,
            rotation: Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2),
            size: Vec2::new(2.0, 1.0),
        }
        .at(Vec3::ZERO)
    }

    fn tube(length: f32) -> AreaShape {
        AreaLightShape {
            kind: LightType::Tube,
            rotation: Quat::IDENTITY,
            size: Vec2::new(length, 0.0),
        }
        .at(Vec3::ZERO)
    }

    fn visibility(
        shape: &AreaShape,
        texel: (Vec3, Vec3),
        blocks: impl Fn(Vec3) -> bool,
    ) -> Option<TexelVisibility> {
        let mut rng = random::stream(&[1, 2, 3]);
        texel_visibility(shape, 10.0, texel, 32, &mut rng, |direction, distance| {
            blocks(texel.0 + direction * distance)
        })
    }

    #[test]
    fn panels_face_down_and_span_their_size() {
        let shape = panel();
        assert!(shape.forward.abs_diff_eq(Vec3::NEG_Y, 1e-6));
        let corners = Aabb::from_points(&shape.corners());
        assert!(corners.min.abs_diff_eq(Vec3::new(-1.0, 0.0, -0.5), 1e-6));
        assert!(corners.max.abs_diff_eq(Vec3::new(1.0, 0.0, 0.5), 1e-6));
        assert!(
            shape
                .point_at(Vec2::ZERO)
                .abs_diff_eq(shape.corners()[0], 1e-6)
        );
        assert!(
            shape
                .point_at(Vec2::ONE)
                .abs_diff_eq(shape.corners()[2], 1e-6)
        );
    }

    #[test]
    fn open_texels_stop_after_the_probes() {
        let below = (Vec3::new(0.3, -2.0, 0.1), Vec3::Y);
        let open = visibility(&panel(), below, |_| false).unwrap();
        assert_eq!((open.fraction, open.rays), (1.0, 5));
        let covered = visibility(&panel(), below, |_| true).unwrap();
        assert_eq!((covered.fraction, covered.rays), (0.0, 5));
        let lit_tube = visibility(&tube(2.0), below, |_| false).unwrap();
        assert_eq!((lit_tube.fraction, lit_tube.rays), (1.0, 3));
    }

    #[test]
    fn half_covered_panels_give_a_penumbra() {
        // Straight below the center, blocking one half of the panel blocks half the light.
        let below = (Vec3::new(0.0, -2.0, 0.0), Vec3::Y);
        let half = visibility(&panel(), below, |point| point.x < 0.0).unwrap();
        assert_eq!(half.rays, 5 + 32);
        assert!((half.fraction - 0.5).abs() < 0.08, "{}", half.fraction);
        // Off to the side, the near half sends more light than the far half.
        let side = (Vec3::new(1.5, -1.0, 0.0), Vec3::Y);
        let near = visibility(&panel(), side, |point| point.x < 0.0).unwrap();
        assert!(near.fraction > 0.6, "{}", near.fraction);
    }

    #[test]
    fn texels_behind_panels_or_out_of_reach_see_nothing() {
        assert_eq!(
            visibility(&panel(), (Vec3::Y, Vec3::NEG_Y), |_| false),
            None
        );
        let up = (Vec3::new(0.0, -2.0, 0.0), Vec3::NEG_Y);
        assert_eq!(visibility(&panel(), up, |_| false), None, "facing away");
        // A tube lights all around; its reach is measured from the nearest point.
        let shape = tube(4.0);
        let above = (Vec3::new(1.9, 1.0, 0.0), Vec3::NEG_Y);
        assert!(visibility(&shape, above, |_| false).is_some());
        let mut rng = random::stream(&[0]);
        let beyond = (Vec3::new(3.5, 0.0, 0.0), Vec3::NEG_X);
        let mut reach = |reach| texel_visibility(&shape, reach, beyond, 8, &mut rng, |_, _| false);
        assert!(reach(1.6).is_some() && reach(1.4).is_none());
        assert!(
            reach(-2.0).is_none(),
            "a radius inside the shape lights nothing"
        );
    }

    #[test]
    fn area_lights_reach_and_face_with_their_shape() {
        let mut light = crate::bake::tests::point_light(1, Vec3::Y * 2.0, 3.0);
        assert_eq!(
            (light.shape_reach(), light.shadow_bits_per_texel()),
            (3.0, 1)
        );
        light.radius = 5.0;
        light.area = Some(AreaLightShape {
            kind: LightType::Tube,
            rotation: Quat::IDENTITY,
            size: Vec2::new(4.0, 0.0),
        });
        assert_eq!(
            (light.shape_reach(), light.shadow_bits_per_texel()),
            (3.0, 4)
        );
        // A wall facing +X at x = 1.5 still sees the tube's far end.
        let wall = [
            Vec3::new(1.5, 0.0, 0.0),
            Vec3::new(1.5, 1.0, 0.0),
            Vec3::new(1.5, 0.0, 1.0),
        ];
        assert!(light.may_face(wall, Vec3::X));
        let far_wall = wall.map(|v| v + Vec3::X * 1.0);
        assert!(!light.may_face(far_wall, Vec3::X));
        // A floor above a downward panel is on its dark side.
        light.area = Some(AreaLightShape {
            kind: LightType::Rectangle,
            rotation: Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2),
            size: Vec2::ONE,
        });
        let floor = [Vec3::ZERO, Vec3::Z, Vec3::X].map(|v| v + Vec3::Y * 3.0);
        assert!(!light.may_face(floor, Vec3::Y));
        assert!(light.may_face(floor.map(|v| v - Vec3::Y * 3.0), Vec3::Y));
    }
}
