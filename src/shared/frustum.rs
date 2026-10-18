//! Whether a camera sees a light's sphere, for the slices that pick lights by what the cameras
//! see.

use bevy::camera::primitives::{Frustum, Sphere};
use bevy::math::Vec3;

/// Whether any of the sphere at `center` with `radius` lies inside `frustum`. Bevy's perspective
/// projections are infinite reverse-Z, so nothing past a camera's `far` is clipped; like Bevy's
/// own culling, this ignores the far plane.
pub fn sees_sphere(frustum: &Frustum, center: Vec3, radius: f32) -> bool {
    let sphere = Sphere {
        center: center.into(),
        radius,
    };
    frustum.intersects_sphere(&sphere, false)
}

#[cfg(test)]
mod tests {
    use bevy::camera::{CameraProjection, PerspectiveProjection};
    use bevy::transform::components::GlobalTransform;

    use super::*;

    fn frustum() -> Frustum {
        let projection = PerspectiveProjection {
            far: 10.0,
            ..Default::default()
        };
        projection.compute_frustum(&GlobalTransform::IDENTITY)
    }

    #[test]
    fn spheres_past_the_far_value_are_seen() {
        let frustum = frustum();
        let beyond = Vec3::new(0.0, 0.0, -20.0);
        assert!(sees_sphere(&frustum, beyond, 5.0));
        let sphere = Sphere {
            center: beyond.into(),
            radius: 5.0,
        };
        assert!(
            !frustum.intersects_sphere(&sphere, true),
            "the far plane would have dropped it"
        );
    }

    #[test]
    fn spheres_behind_or_beside_the_camera_are_not_seen() {
        let frustum = frustum();
        assert!(!sees_sphere(&frustum, Vec3::new(0.0, 0.0, 20.0), 5.0));
        assert!(!sees_sphere(&frustum, Vec3::new(40.0, 0.0, -5.0), 5.0));
        assert!(
            sees_sphere(&frustum, Vec3::new(0.0, 0.0, 3.0), 5.0),
            "it reaches in front"
        );
    }
}
