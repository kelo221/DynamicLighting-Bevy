//! Cube map conventions shared by the baker and the shaders: face order, direction ↔ (face, uv)
//! and texel indexing.
//!
//! These follow WebGPU/Vulkan exactly ("Cube Map Face Selection" in the Vulkan specification),
//! so a texel written here is the texel a `texture_cube` sample in that direction reads:
//!
//! | face | major axis | `sc` | `tc` |
//! |---|---|---|---|
//! | 0 `+X` | `+x` | `-z` | `-y` |
//! | 1 `-X` | `-x` | `+z` | `-y` |
//! | 2 `+Y` | `+y` | `+x` | `+z` |
//! | 3 `-Y` | `-y` | `+x` | `-z` |
//! | 4 `+Z` | `+z` | `+x` | `-y` |
//! | 5 `-Z` | `-z` | `-x` | `-y` |
//!
//! with `u = (sc / |ma| + 1) / 2` and `v = (tc / |ma| + 1) / 2`. Texel `(x, y)` of a face with
//! `resolution` texels per side covers `u ∈ [x, x + 1) / resolution` (`v` likewise, `y = 0` is the
//! first row in memory), and its flat index is `face · resolution² + y · resolution + x`.
//!
//! Ties between equally large components pick X over Y over Z (strictly greater comparisons);
//! the WESL mirror must use the same order.

use bevy::math::{Vec2, Vec3};

/// Number of faces of a cube map.
pub const CUBE_FACE_COUNT: u32 = 6;

/// The face a direction falls on and its `[0, 1]²` coordinates within that face.
pub fn direction_to_face_uv(direction: Vec3) -> (u32, Vec2) {
    let abs = direction.abs();
    let (face, sc, tc, ma) = if abs.x > abs.y && abs.x > abs.z {
        if direction.x > 0.0 {
            (0, -direction.z, -direction.y, abs.x)
        } else {
            (1, direction.z, -direction.y, abs.x)
        }
    } else if abs.y > abs.z {
        if direction.y > 0.0 {
            (2, direction.x, direction.z, abs.y)
        } else {
            (3, direction.x, -direction.z, abs.y)
        }
    } else if direction.z > 0.0 {
        (4, direction.x, -direction.y, abs.z)
    } else {
        (5, -direction.x, -direction.y, abs.z)
    };
    if ma == 0.0 {
        return (face, Vec2::splat(0.5));
    }
    (face, Vec2::new(sc / ma, tc / ma) * 0.5 + 0.5)
}

/// The unit direction through `uv` (in `[0, 1]²`) on `face`; the inverse of
/// [`direction_to_face_uv`].
pub fn face_uv_to_direction(face: u32, uv: Vec2) -> Vec3 {
    let sc = uv.x * 2.0 - 1.0;
    let tc = uv.y * 2.0 - 1.0;
    let direction = match face {
        0 => Vec3::new(1.0, -tc, -sc),
        1 => Vec3::new(-1.0, -tc, sc),
        2 => Vec3::new(sc, 1.0, tc),
        3 => Vec3::new(sc, -1.0, -tc),
        4 => Vec3::new(sc, -tc, 1.0),
        _ => Vec3::new(-sc, -tc, -1.0),
    };
    direction.normalize()
}

/// The direction through the center of texel `(x, y)` on `face`.
pub fn texel_direction(face: u32, x: u32, y: u32, resolution: u32) -> Vec3 {
    let size = resolution as f32;
    face_uv_to_direction(
        face,
        Vec2::new((x as f32 + 0.5) / size, (y as f32 + 0.5) / size),
    )
}

/// Flat index of texel `(x, y)` on `face`: `face · resolution² + y · resolution + x`.
pub fn texel_index(face: u32, x: u32, y: u32, resolution: u32) -> u32 {
    face * resolution * resolution + y * resolution + x
}

/// The texel a direction falls in, as `(face, x, y)`.
pub fn direction_to_texel(direction: Vec3, resolution: u32) -> (u32, u32, u32) {
    let (face, uv) = direction_to_face_uv(direction);
    let max = resolution.saturating_sub(1) as f32;
    let x = (uv.x * resolution as f32).floor().clamp(0.0, max) as u32;
    let y = (uv.y * resolution as f32).floor().clamp(0.0, max) as u32;
    (face, x, y)
}

/// The contents of an index lookup cube (one `u32` per texel, faces in order): every texel holds
/// its own [`texel_index`]. Shaders sample it to turn a direction into a buffer index.
pub fn index_lookup_texels(resolution: u32) -> Vec<u32> {
    (0..CUBE_FACE_COUNT * resolution * resolution).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_land_in_face_centers() {
        let axes = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
        for (face, axis) in axes.into_iter().enumerate() {
            let (found, uv) = direction_to_face_uv(axis);
            assert_eq!(found, face as u32);
            assert!(uv.abs_diff_eq(Vec2::splat(0.5), 1e-6));
            assert!(face_uv_to_direction(face as u32, Vec2::splat(0.5)).abs_diff_eq(axis, 1e-6));
        }
    }

    #[test]
    fn matches_the_vulkan_face_table() {
        // +X: s grows towards -Z, t grows towards -Y.
        let (face, uv) = direction_to_face_uv(Vec3::new(1.0, -0.5, -0.5));
        assert_eq!(face, 0);
        assert!(uv.abs_diff_eq(Vec2::new(0.75, 0.75), 1e-6));
        // +Y: s towards +X, t towards +Z.
        let (face, uv) = direction_to_face_uv(Vec3::new(0.5, 1.0, 0.5));
        assert_eq!(face, 2);
        assert!(uv.abs_diff_eq(Vec2::new(0.75, 0.75), 1e-6));
        // -Y: s towards +X, t towards -Z.
        let (face, uv) = direction_to_face_uv(Vec3::new(0.5, -1.0, -0.5));
        assert_eq!(face, 3);
        assert!(uv.abs_diff_eq(Vec2::new(0.75, 0.75), 1e-6));
        // -Z: s towards -X, t towards -Y.
        let (face, uv) = direction_to_face_uv(Vec3::new(-0.5, -0.5, -1.0));
        assert_eq!(face, 5);
        assert!(uv.abs_diff_eq(Vec2::new(0.75, 0.75), 1e-6));
    }

    #[test]
    fn every_texel_center_round_trips() {
        let resolution = 16;
        for face in 0..CUBE_FACE_COUNT {
            for y in 0..resolution {
                for x in 0..resolution {
                    let direction = texel_direction(face, x, y, resolution);
                    assert!((direction.length() - 1.0).abs() < 1e-5);
                    assert_eq!(direction_to_texel(direction, resolution), (face, x, y));
                    assert_eq!(
                        direction_to_texel(direction * 7.5, resolution),
                        (face, x, y)
                    );
                }
            }
        }
    }

    #[test]
    fn uv_round_trips_inside_faces() {
        for face in 0..CUBE_FACE_COUNT {
            for uv in [
                Vec2::new(0.1, 0.9),
                Vec2::new(0.3, 0.2),
                Vec2::new(0.99, 0.01),
            ] {
                let (found, back) = direction_to_face_uv(face_uv_to_direction(face, uv));
                assert_eq!(found, face);
                assert!(back.abs_diff_eq(uv, 1e-5), "{face} {uv} {back}");
            }
        }
    }

    #[test]
    fn indices_are_face_major_then_rows() {
        assert_eq!(texel_index(0, 0, 0, 64), 0);
        assert_eq!(texel_index(0, 1, 0, 64), 1);
        assert_eq!(texel_index(0, 0, 1, 64), 64);
        assert_eq!(texel_index(1, 0, 0, 64), 4096);
        assert_eq!(texel_index(5, 63, 63, 64), 6 * 4096 - 1);
        let lookup = index_lookup_texels(4);
        assert_eq!(lookup.len(), 96);
        assert_eq!(
            lookup[texel_index(3, 2, 1, 4) as usize],
            texel_index(3, 2, 1, 4)
        );
    }

    #[test]
    fn degenerate_directions_do_not_panic() {
        assert_eq!(direction_to_face_uv(Vec3::ZERO).1, Vec2::splat(0.5));
        let (_, x, y) = direction_to_texel(Vec3::ZERO, 64);
        assert!(x < 64 && y < 64);
    }
}
