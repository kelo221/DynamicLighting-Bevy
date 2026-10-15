//! Henry's `TriangleUvToFull3dStep`: the world position and normal under every texel center
//! of a triangle's UV1 footprint.

use bevy::math::{Vec2, Vec3};

use super::bits::TexelRect;
use super::geometry::uv_barycentrics;
use super::mesh_builder::trace_rect;

/// A texel whose center lies inside the triangle in UV space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceTexel {
    pub x: u32,
    pub y: u32,
    pub position: Vec3,
    /// Interpolated vertex normal, renormalized (the face normal where it degenerates).
    pub normal: Vec3,
}

/// The world-space point and normal at `uv`, or `None` when `uv` is outside the triangle.
///
/// Henry signals "outside" with a (0, 0, 0) position, which also drops texels that really lie
/// at the world origin, and leaves interpolated normals unnormalized; both are fixed here.
pub fn uv_to_surface(
    vertices: [Vec3; 3],
    normals: [Vec3; 3],
    uvs: [Vec2; 3],
    face_normal: Vec3,
    uv: Vec2,
) -> Option<(Vec3, Vec3)> {
    let w = uv_barycentrics(uvs, uv)?;
    let position = vertices[0] * w.x + vertices[1] * w.y + vertices[2] * w.z;
    let normal = (normals[0] * w.x + normals[1] * w.y + normals[2] * w.z)
        .try_normalize()
        .unwrap_or(face_normal);
    Some((position, normal))
}

/// The center of texel `(x, y)` in UV space.
pub fn texel_center(x: u32, y: u32, size: u32) -> Vec2 {
    Vec2::new(
        (x as f32 + 0.5) / size as f32,
        (y as f32 + 0.5) / size as f32,
    )
}

/// Every texel of a `size`² map whose center lies inside the triangle, row by row.
pub fn triangle_texels(
    vertices: [Vec3; 3],
    normals: [Vec3; 3],
    uvs: [Vec2; 3],
    face_normal: Vec3,
    size: u32,
) -> Vec<SurfaceTexel> {
    let rect: TexelRect = trace_rect(uvs, size);
    let mut texels = Vec::new();
    for y in rect.min_y..=rect.max_y {
        for x in rect.min_x..=rect.max_x {
            let uv = texel_center(x, y, size);
            if let Some((position, normal)) = uv_to_surface(vertices, normals, uvs, face_normal, uv)
            {
                texels.push(SurfaceTexel {
                    x,
                    y,
                    position,
                    normal,
                });
            }
        }
    }
    texels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texels_interpolate_position_and_normal() {
        // A right triangle covering the lower-left half of UV space, lying in the XZ plane.
        let vertices = [
            Vec3::ZERO,
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 4.0),
        ];
        let normals = [Vec3::Y, Vec3::Y * 2.0, Vec3::new(0.0, 1.0, 1.0)];
        let uvs = [Vec2::ZERO, Vec2::X, Vec2::Y];
        let texels = triangle_texels(vertices, normals, uvs, Vec3::Y, 4);
        // Centers (x + 0.5, y + 0.5) / 4 with x + y + 1 <= 4: 4 + 3 + 2 + 1 texels.
        assert_eq!(texels.len(), 10);
        let first = texels[0];
        assert_eq!((first.x, first.y), (0, 0));
        assert!(first.position.abs_diff_eq(Vec3::new(0.5, 0.0, 0.5), 1e-5));
        for texel in &texels {
            assert!((texel.normal.length() - 1.0).abs() < 1e-5);
            assert!(texel.x + texel.y <= 3);
        }
    }

    #[test]
    fn the_world_origin_is_a_valid_texel() {
        let vertices = [
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::ZERO,
        ];
        let uvs = [Vec2::ZERO, Vec2::X, Vec2::new(0.5, 1.0)];
        let hit = uv_to_surface(vertices, [Vec3::Y; 3], uvs, Vec3::Y, Vec2::new(0.5, 1.0));
        assert_eq!(hit, Some((Vec3::ZERO, Vec3::Y)));
        assert!(uv_to_surface(vertices, [Vec3::Y; 3], uvs, Vec3::Y, Vec2::new(0.0, 1.0)).is_none());
    }

    #[test]
    fn opposing_normals_fall_back_to_the_face_normal() {
        let vertices = [Vec3::ZERO, Vec3::X, Vec3::Z];
        let normals = [Vec3::Y, -Vec3::Y, Vec3::ZERO];
        let uvs = [Vec2::ZERO, Vec2::X, Vec2::Y];
        let (_, normal) =
            uv_to_surface(vertices, normals, uvs, Vec3::NEG_Y, Vec2::new(0.5, 0.0)).unwrap();
        assert_eq!(normal, Vec3::NEG_Y);
    }
}
