//! Where the six cameras of a shadow cubemap look, and which lights get a cubemap.

use bevy::math::Vec3;

/// Texels per side of a shadow cubemap face (Henry's 512).
pub const SHADOW_CUBEMAP_RESOLUTION: u32 = 512;

/// Near plane of the face cameras. Occluders closer to the light than this are clipped.
pub const SHADOW_NEAR_PLANE: f32 = 0.01;

/// Forward and up of the camera that renders cube face `face`, in WebGPU face order
/// (`+X, -X, +Y, -Y, +Z, -Z`; see `shared::cube`). Cube faces are left-handed, so the rendered
/// image is mirrored horizontally when it is written into the face.
pub fn face_orientation(face: u32) -> (Vec3, Vec3) {
    match face {
        0 => (Vec3::X, Vec3::Y),
        1 => (Vec3::NEG_X, Vec3::Y),
        2 => (Vec3::Y, Vec3::NEG_Z),
        3 => (Vec3::NEG_Y, Vec3::Z),
        4 => (Vec3::Z, Vec3::Y),
        _ => (Vec3::NEG_Z, Vec3::Y),
    }
}

/// The lights that get shadow cubemaps: those nearest to `viewer` first (ties by key), at most
/// `budget`. Without a viewer the order is by key alone.
pub fn select_shadow_lights<K: Copy + Ord>(
    mut candidates: Vec<(K, Vec3)>,
    viewer: Option<Vec3>,
    budget: usize,
) -> Vec<K> {
    let distance = |position: Vec3| viewer.map_or(0.0, |viewer| viewer.distance_squared(position));
    candidates.sort_by(|(a, pa), (b, pb)| {
        distance(*pa)
            .total_cmp(&distance(*pb))
            .then_with(|| a.cmp(b))
    });
    candidates.truncate(budget);
    candidates.into_iter().map(|(key, _)| key).collect()
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec2;
    use bevy::transform::components::Transform;

    use super::*;
    use crate::shared::cube::{CUBE_FACE_COUNT, face_uv_to_direction};

    /// The direction a camera with `transform` and a 90° square view sees at image coordinates
    /// `(u, v)`, `v` growing downwards like texture rows.
    fn image_direction(transform: &Transform, u: f32, v: f32) -> Vec3 {
        let (right, up, forward) = (
            transform.right().as_vec3(),
            transform.up().as_vec3(),
            transform.forward().as_vec3(),
        );
        (forward + right * (2.0 * u - 1.0) + up * (1.0 - 2.0 * v)).normalize()
    }

    #[test]
    fn mirrored_face_images_match_the_cube_convention() {
        for face in 0..CUBE_FACE_COUNT {
            let (forward, up) = face_orientation(face);
            let camera = Transform::default().looking_to(forward, up);
            for uv in [
                Vec2::new(0.1, 0.2),
                Vec2::new(0.9, 0.3),
                Vec2::new(0.5, 0.95),
            ] {
                let expected = face_uv_to_direction(face, uv).normalize();
                let seen = image_direction(&camera, 1.0 - uv.x, uv.y);
                assert!(
                    seen.abs_diff_eq(expected, 1e-5),
                    "face {face} at {uv}: {seen} != {expected}"
                );
            }
        }
    }

    #[test]
    fn nearest_lights_win_within_the_budget() {
        let lights = vec![
            (3, Vec3::new(9.0, 0.0, 0.0)),
            (1, Vec3::new(5.0, 0.0, 0.0)),
            (2, Vec3::new(1.0, 0.0, 0.0)),
            (0, Vec3::new(5.0, 0.0, 0.0)),
        ];
        assert_eq!(
            select_shadow_lights(lights.clone(), Some(Vec3::ZERO), 3),
            [2, 0, 1]
        );
        assert_eq!(select_shadow_lights(lights, None, 2), [0, 1]);
    }
}
