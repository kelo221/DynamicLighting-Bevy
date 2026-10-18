//! The sun's realtime shadow cascades: orthographic views along the sun's rays, each over a
//! slice of the camera's view. A cascade is a square around its slice's bounding sphere whose
//! center snaps to a grid of [`SNAP_TEXELS`] texels; its depth reaches from the sunward end of
//! the casters over it to the far side of the sphere, snapped to the same step. So a cascade
//! holds still while the camera moves less than a step, and when it moves it moves by whole
//! texels, which keeps the shadows of still casters from shimmering.

use bevy::math::{IVec2, Mat4, Vec3, Vec4Swizzles, proj};
use bevy::transform::components::{GlobalTransform, Transform};

use super::casters::Caster;
use super::slots::CasterChange;
use super::sun_mirror::SUN_CASCADE_MARGIN;
use crate::shared::{GpuSunCascade, SUN_CASCADE_COUNT};

/// Texels per side of a cascade.
pub const SUN_CASCADE_RESOLUTION: u32 = 1024;
/// Cascades move in steps of this many texels.
pub const SNAP_TEXELS: u32 = 64;
/// How much of the split depths follow the logarithmic scheme, the rest the uniform one (the
/// practical split scheme of Zhang, Sun, Xu and Lun 2006).
const SPLIT_LAMBDA: f32 = 0.75;
/// The split schemes start at this view depth or the camera's near plane, whichever is farther.
const MIN_SPLIT_NEAR: f32 = 0.1;
/// Half sides are rounded up to a multiple of this, in meters, so the rounding errors of a
/// turning camera don't change them.
const HALF_QUANTUM: f32 = 1.0 / 64.0;
/// Depth bias of a receiver: this many meters plus [`SUN_DEPTH_BIAS_TEXELS`] texels.
pub const SUN_DEPTH_BIAS: f32 = 0.05;
pub const SUN_DEPTH_BIAS_TEXELS: f32 = 2.0;

/// The camera the cascades cover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CascadeViewer {
    pub world_from_view: Mat4,
    pub clip_from_view: Mat4,
}

impl CascadeViewer {
    /// The view-space point at normalized device coordinates `ndc` (reverse Z: 1 is near).
    fn unproject(&self, ndc: Vec3) -> Vec3 {
        let point = self.clip_from_view.inverse() * ndc.extend(1.0);
        point.xyz() / point.w
    }

    /// View depth of the near plane.
    pub(super) fn near(&self) -> f32 {
        (-self.unproject(Vec3::Z).z).max(0.0)
    }

    /// World corners of the view between view depths `from` and `to`: the four corner rays at
    /// `from`, then at `to`. The rays are lines through the near plane, so this holds for
    /// perspective and orthographic cameras alike.
    pub(super) fn slice(&self, from: f32, to: f32) -> [Vec3; 8] {
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        core::array::from_fn(|index| {
            let (x, y) = corners[index % 4];
            let a = self.unproject(Vec3::new(x, y, 1.0));
            let b = self.unproject(Vec3::new(x, y, 0.5));
            let depth = if index < 4 { from } else { to };
            let t = (depth + a.z) / (a.z - b.z);
            self.world_from_view.transform_point3(a.lerp(b, t))
        })
    }
}

/// View depths where the cascades begin and end: the camera's `near` plane, then the practical
/// split scheme's splits up to `distance`.
pub fn split_depths(near: f32, distance: f32) -> [f32; SUN_CASCADE_COUNT + 1] {
    let start = near.max(MIN_SPLIT_NEAR);
    let end = distance.max(start * 2.0);
    let count = SUN_CASCADE_COUNT as f32;
    core::array::from_fn(|index| {
        if index == 0 {
            return near;
        }
        let share = index as f32 / count;
        let logarithmic = start * (end / start).powf(share);
        let uniform = start + (end - start) * share;
        SPLIT_LAMBDA * logarithmic + (1.0 - SPLIT_LAMBDA) * uniform
    })
}

/// The smallest sphere around `corners` centered between the centers of their near and far
/// faces.
fn bounding_sphere(corners: &[Vec3; 8]) -> (Vec3, f32) {
    let near = corners[..4].iter().copied().sum::<Vec3>() / 4.0;
    let far = corners[4..].iter().copied().sum::<Vec3>() / 4.0;
    let reach = |t: f32| {
        let center = near.lerp(far, t);
        corners
            .iter()
            .map(|corner| corner.distance(center))
            .fold(0.0, f32::max)
    };
    // The farthest corner's distance is convex along the segment.
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..48 {
        let (a, b) = (low + (high - low) / 3.0, high - (high - low) / 3.0);
        if reach(a) <= reach(b) {
            high = b;
        } else {
            low = a;
        }
    }
    let t = (low + high) * 0.5;
    (near.lerp(far, t), reach(t))
}

/// The frame of the sun's rays: `forward` along them, away from the sun.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunBasis {
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
}

impl SunBasis {
    /// The frame of a sun towards unit `to_sun`.
    pub fn new(to_sun: Vec3) -> Self {
        let forward = -to_sun.normalize();
        let hint = if forward.y.abs() > 0.999 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let right = forward.cross(hint).normalize();
        Self {
            right,
            up: right.cross(forward),
            forward,
        }
    }

    /// `point` along `right`, `up` and `forward`.
    pub fn local(&self, point: Vec3) -> Vec3 {
        Vec3::new(
            point.dot(self.right),
            point.dot(self.up),
            point.dot(self.forward),
        )
    }
}

/// What a cascade is rendered from; when it differs from the last rendering, the cascade renders
/// again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CascadePose {
    pub basis: SunBasis,
    /// Half the side of the square the cascade covers.
    pub half: f32,
    /// Its center along `right` and `up`, in steps.
    pub center: IVec2,
    /// Its sunward and far ends along the rays, in steps.
    pub depth: IVec2,
    /// Only meshes without bake data are drawn, because the sun uses its baked shadows.
    pub moving_only: bool,
}

impl CascadePose {
    pub fn texel_size(&self) -> f32 {
        2.0 * self.half / SUN_CASCADE_RESOLUTION as f32
    }

    /// The snapping step, [`SNAP_TEXELS`] texels.
    pub fn step(&self) -> f32 {
        self.texel_size() * SNAP_TEXELS as f32
    }

    fn center_point(&self) -> (f32, f32) {
        let step = self.step();
        (self.center.x as f32 * step, self.center.y as f32 * step)
    }

    fn near(&self) -> f32 {
        self.depth.x as f32 * self.step()
    }

    fn far(&self) -> f32 {
        self.depth.y as f32 * self.step()
    }

    /// Meters from the sunward end to the far end.
    pub fn range(&self) -> f32 {
        self.far() - self.near()
    }

    /// The cascade's view: at its center on the sunward end, looking along the rays, with
    /// reverse Z like Bevy's orthographic projection (depth 1 at the sunward end).
    pub fn view(&self) -> (GlobalTransform, Mat4) {
        let (x, y) = self.center_point();
        let basis = self.basis;
        let origin = basis.right * x + basis.up * y + basis.forward * self.near();
        let transform = Transform::from_translation(origin).looking_to(basis.forward, basis.up);
        let half = self.half;
        let projection = proj::orthographic(-half, half, -half, half, self.range(), 0.0);
        (transform.into(), projection)
    }

    /// Whether a sphere can cast a shadow into the cascade or receive one in it.
    pub fn touches(&self, center: Vec3, radius: f32) -> bool {
        let local = self.basis.local(center);
        let (x, y) = self.center_point();
        let reach = self.half + radius;
        (local.x - x).abs() <= reach
            && (local.y - y).abs() <= reach
            && local.z - radius <= self.far()
    }

    /// Whether the cascade draws `caster`.
    pub fn draws(&self, caster: Caster) -> bool {
        if !caster.shown || (self.moving_only && caster.baked) {
            return false;
        }
        caster
            .bounds
            .is_none_or(|(center, radius)| self.touches(center, radius))
    }

    /// The cascade as the lit shader reads it.
    pub fn gpu(&self) -> GpuSunCascade {
        let (x, y) = self.center_point();
        let scale = 0.5 / self.half;
        let range = self.range();
        let row = |axis: Vec3, factor: f32, offset: f32| {
            let axis = axis * factor;
            [axis.x, axis.y, axis.z, offset]
        };
        let basis = self.basis;
        GpuSunCascade {
            u: row(basis.right, scale, 0.5 - x * scale),
            v: row(basis.up, -scale, 0.5 + y * scale),
            depth: row(basis.forward, 1.0 / range, -self.near() / range),
            texel_size: self.texel_size(),
            depth_bias: (SUN_DEPTH_BIAS + SUN_DEPTH_BIAS_TEXELS * self.texel_size()) / range,
            _padding: [0.0; 2],
        }
    }
}

/// The cascades over `viewer`'s view up to view depth `distance`, for a sun towards unit
/// `to_sun`. `casters` are the bounding spheres of what the cascades may draw; each cascade's
/// depth reaches over those over it, from their sunward end to their far end or the far side of
/// the last slice, whichever comes first. So a receiver the cascades shadow that lies past a
/// cascade's far end lies behind every caster it holds.
pub fn fit_cascades(
    viewer: &CascadeViewer,
    to_sun: Vec3,
    distance: f32,
    moving_only: bool,
    casters: &[(Vec3, f32)],
) -> [CascadePose; SUN_CASCADE_COUNT] {
    let basis = SunBasis::new(to_sun);
    let depths = split_depths(viewer.near(), distance);
    let mut spheres = [(Vec3::ZERO, 0.0); SUN_CASCADE_COUNT];
    for (index, sphere) in spheres.iter_mut().enumerate() {
        let (center, radius) = bounding_sphere(&viewer.slice(depths[index], depths[index + 1]));
        // Wide enough that the sphere stays inside the rim while its center snaps to the
        // nearest step: half a step is a sixteenth of the half side.
        let snap = SNAP_TEXELS as f32 / SUN_CASCADE_RESOLUTION as f32;
        let fit = 1.0 - SUN_CASCADE_MARGIN - snap;
        *sphere = (
            basis.local(center),
            (radius / fit / HALF_QUANTUM).ceil() * HALF_QUANTUM,
        );
    }
    let last = spheres
        .iter()
        .map(|(local, half)| local.z + half)
        .fold(f32::MIN, f32::max);
    spheres.map(|(local, half)| {
        let step = 2.0 * half / SUN_CASCADE_RESOLUTION as f32 * SNAP_TEXELS as f32;
        let grid = IVec2::new(
            (local.x / step).round() as i32,
            (local.y / step).round() as i32,
        );
        let (x, y) = (grid.x as f32 * step, grid.y as f32 * step);
        let (sunward, far) = casters
            .iter()
            .map(|&(center, radius)| (basis.local(center), radius))
            .filter(|(at, radius)| {
                (at.x - x).abs() <= half + radius && (at.y - y).abs() <= half + radius
            })
            .fold(
                (local.z - half, local.z + half),
                |(sunward, far), (at, radius)| {
                    (
                        sunward.min(at.z - radius),
                        far.max((at.z + radius).min(last)),
                    )
                },
            );
        CascadePose {
            basis,
            half,
            center: grid,
            depth: IVec2::new((sunward / step).floor() as i32, (far / step).ceil() as i32),
            moving_only,
        }
    })
}

/// The cascades to render this frame, one bit each, recording them as rendered: every cascade
/// on `reset`, those whose pose differs from their last rendering, those a caster change
/// touches and those in `missed`, whose last rendering lacked some of their casters.
pub fn plan_cascades(
    rendered: &mut [Option<CascadePose>; SUN_CASCADE_COUNT],
    poses: &[CascadePose; SUN_CASCADE_COUNT],
    changes: &[CasterChange],
    missed: u8,
    reset: bool,
) -> u8 {
    let mut cascades = 0;
    for (index, (last, pose)) in rendered.iter_mut().zip(poses).enumerate() {
        let touched = changes.iter().any(|change| {
            !(pose.moving_only && change.baked) && pose.touches(change.center, change.radius)
        });
        if reset || *last != Some(*pose) || touched || missed & (1 << index) != 0 {
            cascades |= 1 << index;
        }
        *last = Some(*pose);
    }
    cascades
}
