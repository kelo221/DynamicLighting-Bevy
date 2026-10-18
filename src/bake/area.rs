//! Soft shadows of rectangle and tube lights: per texel, shadow rays to points spread over the
//! shape, each weighted by how much light that part of the shape sends to the texel. Distance
//! cubes and bounce light still come from the light's center.

use bevy::math::{Vec2, Vec3};

use crate::effects::EffectRng;
use crate::light::area::AreaShape;

use super::mesh_bake::NDOTL_TOLERANCE;
use super::spread::spread;

/// Shadow bits per texel of area lights: 16 levels of visibility.
pub const AREA_SHADOW_BITS: u32 = 4;
/// Shape points probed before deciding whether a texel lies in a penumbra: a rectangle's corners
/// and center, a tube's ends and middle. In a penumbra an occluder usually hides one of them, but
/// a thin one (a pole, a railing) can pass between them all; [`AreaSampling::minimum`] catches
/// those.
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

/// What one texel sees of an area light or a sun's disk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TexelVisibility {
    /// The share of the light reaching the texel unblocked, 0 to 1.
    pub fraction: f32,
    pub rays: u32,
}

/// Shadow rays per texel to points spread over an area light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AreaSampling {
    /// Rays where the texel may lie in a penumbra.
    pub samples: u32,
    /// The first rays of those that every texel takes after the probes (at most `samples`).
    pub minimum: u32,
}

/// How much of `shape` reaches a texel at `position` with unit `normal`.
///
/// Shadow rays first go to the probe points, then to the first `sampling.minimum` points spread
/// over the shape ([`spread`], shifted by `rng`); when they all agree the texel is fully lit or
/// fully shadowed. Otherwise the spread continues to `sampling.samples` points, each weighted by
/// the light that part of the shape sends: receiver cosine (with Henry's facing tolerance) ×
/// emitter cosine / distance². `None` when the texel is farther than `reach` from the shape (or
/// `reach` isn't positive), the shape is below its horizon, or it is behind a rectangle.
/// `occluded(direction, distance)` traces one ray from the texel.
pub fn texel_visibility(
    shape: &AreaShape,
    reach: f32,
    (position, normal): (Vec3, Vec3),
    sampling: AreaSampling,
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
    let trace = |uv: Vec2| {
        let (direction, distance, weight) = toward(shape, position, normal, uv)?;
        rays += 1;
        Some((weight, !occluded(direction, (distance - END_GAP).max(0.0))))
    };
    let probes = if shape.rectangle {
        &RECTANGLE_PROBES[..]
    } else {
        &TUBE_PROBES[..]
    };
    let offset = Vec2::new(rng.next_f32(), rng.next_f32());
    let spread_point = |taken| spread(taken, offset, shape.rectangle);
    let fraction = adaptive_share(probes, sampling, spread_point, trace);
    fraction.map(|fraction| TexelVisibility { fraction, rays })
}

/// The lit share of a light seen through shadow rays: first to the `probes`, then to the first
/// `sampling.minimum` spread points (`spread_point(i)`); when they all agree the share is 0 or 1.
/// Otherwise the spread continues to `sampling.samples` points and the share is their
/// weighted average. The probes only decide whether to sample; the share comes from the evenly
/// spread points. `trace(point)` gives a ray's weight and whether it is unblocked, or `None`
/// when that point sends no light; `None` overall when no point does.
pub fn adaptive_share<P: Copy>(
    probes: &[P],
    sampling: AreaSampling,
    spread_point: impl Fn(u32) -> P,
    mut trace: impl FnMut(P) -> Option<(f32, bool)>,
) -> Option<f32> {
    let (mut any_lit, mut any_blocked) = (false, false);
    for (_, lit) in probes.iter().filter_map(|&point| trace(point)) {
        any_lit |= lit;
        any_blocked |= !lit;
    }
    let (mut lit, mut total) = (0.0, 0.0);
    for taken in 0..sampling.samples {
        if taken == sampling.minimum && any_lit != any_blocked {
            break;
        }
        if let Some((weight, unblocked)) = trace(spread_point(taken)) {
            total += weight;
            lit += if unblocked { weight } else { 0.0 };
            any_lit |= unblocked;
            any_blocked |= !unblocked;
        }
    }
    if any_lit != any_blocked {
        Some(if any_lit { 1.0 } else { 0.0 })
    } else {
        (total > 0.0).then(|| lit / total)
    }
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

    /// 32 samples after `minimum`; `blocks` tests the point a ray reaches on the shape.
    fn visibility_with(
        shape: &AreaShape,
        texel: (Vec3, Vec3),
        minimum: u32,
        blocks: impl Fn(Vec3) -> bool,
    ) -> Option<TexelVisibility> {
        let mut rng = random::stream(&[1, 2, 3]);
        let sampling = AreaSampling {
            samples: 32,
            minimum,
        };
        texel_visibility(
            shape,
            10.0,
            texel,
            sampling,
            &mut rng,
            |direction, distance| blocks(texel.0 + direction * distance),
        )
    }

    fn visibility(
        shape: &AreaShape,
        texel: (Vec3, Vec3),
        blocks: impl Fn(Vec3) -> bool,
    ) -> Option<TexelVisibility> {
        visibility_with(shape, texel, 0, blocks)
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
        // The minimum samples come on top of the probes, and no more follow when all agree.
        let open = visibility_with(&panel(), below, 16, |_| false).unwrap();
        assert_eq!((open.fraction, open.rays), (1.0, 5 + 16));
        let covered = visibility_with(&tube(2.0), below, 16, |_| true).unwrap();
        assert_eq!((covered.fraction, covered.rays), (0.0, 3 + 16));
        let capped = visibility_with(&panel(), below, 100, |_| false).unwrap();
        assert_eq!((capped.fraction, capped.rays), (1.0, 5 + 32));
    }

    #[test]
    fn minimum_samples_catch_occluders_between_the_probes() {
        // A bar across the panel's depth hides 15% of its width but none of the probes; 16
        // spread points see any bar wider than 2/16.
        let below = (Vec3::new(0.0, -2.0, 0.0), Vec3::Y);
        let bar = |point: Vec3| (-0.7..-0.4).contains(&point.x);
        let probes_only = visibility(&panel(), below, bar).unwrap();
        assert_eq!((probes_only.fraction, probes_only.rays), (1.0, 5));
        let spread = visibility_with(&panel(), below, 16, bar).unwrap();
        assert_eq!(spread.rays, 5 + 32, "a penumbra takes every sample");
        assert!(
            spread.fraction > 0.7 && spread.fraction < 0.95,
            "{}",
            spread.fraction
        );
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
        let sampling = AreaSampling {
            samples: 8,
            minimum: 0,
        };
        let mut reach =
            |reach| texel_visibility(&shape, reach, beyond, sampling, &mut rng, |_, _| false);
        assert!(reach(1.6).is_some() && reach(1.4).is_none());
        assert!(
            reach(-2.0).is_none(),
            "a radius inside the shape lights nothing"
        );
    }
}
