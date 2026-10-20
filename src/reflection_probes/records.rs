//! A probe's box and the records the lit shader reads: which probe holds a point, and the
//! parallax-corrected reflection direction inside it.

use bevy::math::Affine3A;
use bevy::prelude::*;

use crate::environment::sh::{SH_COUNT, to_gpu};
use crate::shared::{GpuReflectionProbe, GpuReflectionProbes, MAX_REFLECTION_PROBES};

/// A probe's oriented box: its local ±1 cube under `world_from_box`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbeBox {
    pub world_from_box: Affine3A,
}

impl ProbeBox {
    /// The box of a probe entity: centred at its translation, turned by its rotation, with its
    /// scale as half extents. `None` for a degenerate or non-finite transform.
    pub fn new(transform: &GlobalTransform) -> Option<Self> {
        let world_from_box = transform.affine();
        let determinant = world_from_box.matrix3.determinant();
        (determinant.is_finite() && determinant.abs() > 1e-12).then_some(Self { world_from_box })
    }

    /// The box's volume, which orders overlapping probes.
    pub fn volume(&self) -> f32 {
        8.0 * self.world_from_box.matrix3.determinant().abs()
    }

    /// The rows of the world-to-box transform, as the shader reads them.
    pub fn rows(&self) -> [[f32; 4]; 3] {
        let box_from_world = self.world_from_box.inverse();
        let (m, t) = (box_from_world.matrix3, box_from_world.translation);
        [0, 1, 2].map(|row| [m.x_axis[row], m.y_axis[row], m.z_axis[row], t[row]])
    }
}

/// A captured probe, in world space.
#[derive(Clone, Debug, PartialEq)]
pub struct ProbeRecord {
    pub bounds: ProbeBox,
    pub capture: Vec3,
    /// Its cube in the reflection cube array; the sky is cube 0.
    pub layer: u32,
    /// Irradiance over π, already scaled by the probe's `diffuse_intensity`.
    pub irradiance: [Vec3; SH_COUNT],
}

/// The records of `probes`, smallest box first, at most [`MAX_REFLECTION_PROBES`]; equal boxes
/// keep their order. Returns the records and how many probes didn't fit.
pub fn pack_probes(probes: &[ProbeRecord]) -> (GpuReflectionProbes, usize) {
    let mut order: Vec<&ProbeRecord> = probes.iter().collect();
    order.sort_by(|a, b| a.bounds.volume().total_cmp(&b.bounds.volume()));
    let mut packed = GpuReflectionProbes::default();
    let kept = order.len().min(MAX_REFLECTION_PROBES);
    for (record, probe) in packed.probes.iter_mut().zip(&order) {
        let [x, y, z] = probe.bounds.rows();
        *record = GpuReflectionProbe {
            x,
            y,
            z,
            capture: probe.capture.to_array(),
            layer: probe.layer,
            irradiance: to_gpu(&probe.irradiance),
        };
    }
    packed.count = kept as u32;
    (packed, probes.len() - kept)
}

/// `world` in a record's box space.
pub fn box_point(probe: &GpuReflectionProbe, world: Vec3) -> Vec3 {
    let row = |r: [f32; 4]| Vec3::from_slice(&r[..3]).dot(world) + r[3];
    Vec3::new(row(probe.x), row(probe.y), row(probe.z))
}

/// `direction` in a record's box space.
fn box_direction(probe: &GpuReflectionProbe, direction: Vec3) -> Vec3 {
    let row = |r: [f32; 4]| Vec3::from_slice(&r[..3]).dot(direction);
    Vec3::new(row(probe.x), row(probe.y), row(probe.z))
}

/// How far past its sides, as a share of its half extents, a box still holds points, so walls
/// that lie on its sides use it everywhere instead of flickering between it and the next.
pub const HOLD_MARGIN: f32 = 0.01;

/// The probe whose box holds `world`, the first in record order (the smallest box); `None`
/// outside every box. `probes.wesl` mirrors it.
pub fn probe_at(probes: &GpuReflectionProbes, world: Vec3) -> Option<usize> {
    let count = (probes.count as usize).min(MAX_REFLECTION_PROBES);
    probes.probes[..count]
        .iter()
        .position(|probe| box_point(probe, world).abs().max_element() <= 1.0 + HOLD_MARGIN)
}

/// The direction to sample a probe's cube in for a ray from `world` along `direction`: from the
/// capture point to where the ray leaves the box (box projection). `world` must lie in the box.
pub fn box_projected(probe: &GpuReflectionProbe, world: Vec3, direction: Vec3) -> Vec3 {
    let start = box_point(probe, world);
    let step = box_direction(probe, direction);
    // Per axis, the distance along the ray to the face it moves toward; the nearest one is
    // where the ray leaves. Axes it doesn't move along never end it.
    let exit = |p: f32, d: f32| {
        if d.abs() < 1e-8 {
            f32::INFINITY
        } else {
            (d.signum() - p) / d
        }
    };
    let t = exit(start.x, step.x)
        .min(exit(start.y, step.y))
        .min(exit(start.z, step.z))
        .max(0.0);
    world + direction * t - Vec3::from_array(probe.capture)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(center: Vec3, half: Vec3, capture: Vec3, layer: u32) -> ProbeRecord {
        let transform = GlobalTransform::from(Transform::from_translation(center).with_scale(half));
        ProbeRecord {
            bounds: ProbeBox::new(&transform).unwrap(),
            capture,
            layer,
            irradiance: [Vec3::ZERO; SH_COUNT],
        }
    }

    #[test]
    fn the_smallest_box_holding_a_point_wins() {
        let room = record(Vec3::ZERO, Vec3::new(5.0, 3.0, 5.0), Vec3::ZERO, 1);
        let alcove = record(Vec3::new(4.0, 0.0, 0.0), Vec3::ONE, Vec3::X * 4.0, 2);
        let (packed, dropped) = pack_probes(&[room, alcove]);
        assert_eq!((packed.count, dropped), (2, 0));
        assert_eq!(packed.probes[0].layer, 2, "smallest first");
        let layer = |p: Vec3| probe_at(&packed, p).map(|i| packed.probes[i].layer);
        assert_eq!(layer(Vec3::new(4.5, 0.5, 0.0)), Some(2));
        assert_eq!(layer(Vec3::new(-4.0, 0.0, 0.0)), Some(1));
        assert_eq!(layer(Vec3::new(0.0, 3.5, 0.0)), None, "outside every box");
        assert_eq!(layer(Vec3::new(-5.0, 3.0, 0.0)), Some(1), "on its sides");
        assert_eq!(layer(Vec3::new(-5.0, 3.0 * 1.02, 0.0)), None);
    }

    #[test]
    fn turned_boxes_hold_their_own_corners() {
        let transform = GlobalTransform::from(
            Transform::from_xyz(1.0, 2.0, 3.0)
                .with_rotation(Quat::from_rotation_y(0.7))
                .with_scale(Vec3::new(2.0, 1.0, 0.5)),
        );
        let bounds = ProbeBox::new(&transform).unwrap();
        assert!((bounds.volume() - 8.0).abs() < 1e-4);
        let probe = GpuReflectionProbe {
            x: bounds.rows()[0],
            y: bounds.rows()[1],
            z: bounds.rows()[2],
            ..default()
        };
        let corner = transform.transform_point(Vec3::new(0.99, -0.99, 0.99));
        assert!(box_point(&probe, corner).abs().max_element() <= 1.0);
        let beyond = transform.transform_point(Vec3::new(1.01, 0.0, 0.0));
        assert!(box_point(&probe, beyond).abs().max_element() > 1.0);
        assert!(ProbeBox::new(&GlobalTransform::from_scale(Vec3::ZERO)).is_none());
    }

    #[test]
    fn rays_from_off_center_points_hit_the_expected_wall() {
        let probe = record(
            Vec3::ZERO,
            Vec3::new(4.0, 2.0, 3.0),
            Vec3::new(0.0, 0.5, 0.0),
            1,
        );
        let (packed, _) = pack_probes(&[probe]);
        let probe = &packed.probes[0];
        // From 1 m right of the center straight at the +X wall (x = 4): the hit is (4, 0, 0),
        // seen from the capture point at (0, 0.5, 0).
        let toward = box_projected(probe, Vec3::X, Vec3::X);
        assert!(
            toward.abs_diff_eq(Vec3::new(4.0, -0.5, 0.0), 1e-5),
            "{toward}"
        );
        // A diagonal ray leaves through the nearer wall: from (3, 0, 0) along (1, 0, 1) it hits
        // x = 4 first, at z = 1.
        let diagonal = box_projected(probe, Vec3::X * 3.0, Vec3::new(1.0, 0.0, 1.0));
        assert!(
            diagonal.abs_diff_eq(Vec3::new(4.0, -0.5, 1.0), 1e-5),
            "{diagonal}"
        );
        // At the capture point the correction changes nothing but the length.
        let up = box_projected(probe, Vec3::new(0.0, 0.5, 0.0), Vec3::Y);
        assert!(up.normalize().abs_diff_eq(Vec3::Y, 1e-5));
    }

    #[test]
    fn probes_past_the_limit_are_dropped_largest_first() {
        let probes: Vec<_> = (0..MAX_REFLECTION_PROBES + 2)
            .map(|i| {
                record(
                    Vec3::ZERO,
                    Vec3::splat(1.0 + i as f32),
                    Vec3::ZERO,
                    i as u32 + 1,
                )
            })
            .collect();
        let (packed, dropped) = pack_probes(&probes);
        assert_eq!((packed.count as usize, dropped), (MAX_REFLECTION_PROBES, 2));
        assert_eq!(packed.probes[MAX_REFLECTION_PROBES - 1].layer, 16);
    }
}
