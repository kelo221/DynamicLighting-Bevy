//! Henry's `TriangleUvToFull3dStep`: the world position and normal under every texel center
//! of a triangle's UV1 footprint.

use bevy::math::{Vec2, Vec3};

use super::bits::TexelRect;
use super::geometry::{closest_point_on_triangle, uv_barycentrics};
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

/// The texels of a `size`² map that the triangle touches in UV space although their centers lie
/// outside it, skipping those `owned` says some triangle's center sample already covers. Each is
/// placed at the triangle's point nearest the texel center. A triangle narrower than a texel,
/// like the strips of a thin pipe, owns few centers or none, and its texels would stay unlit.
pub fn touched_texels(
    vertices: [Vec3; 3],
    normals: [Vec3; 3],
    uvs: [Vec2; 3],
    face_normal: Vec3,
    size: u32,
    owned: impl Fn(u32, u32) -> bool,
) -> Vec<SurfaceTexel> {
    let s = size as f32;
    let (min, max) = (
        uvs[0].min(uvs[1]).min(uvs[2]),
        uvs[0].max(uvs[1]).max(uvs[2]),
    );
    let first = (min * s).floor().max(Vec2::ZERO);
    let last = (max * s).floor().min(Vec2::splat(s - 1.0));
    let half = 0.5 / s;
    let mut texels = Vec::new();
    for y in first.y as u32..=last.y as u32 {
        for x in first.x as u32..=last.x as u32 {
            if owned(x, y) {
                continue;
            }
            let center = texel_center(x, y, size);
            let Some(w) = nearest_weights(uvs, center) else {
                continue;
            };
            let nearest = uvs[0] * w.x + uvs[1] * w.y + uvs[2] * w.z;
            if (nearest - center).abs().max_element() > half {
                continue;
            }
            let position = vertices[0] * w.x + vertices[1] * w.y + vertices[2] * w.z;
            let normal = (normals[0] * w.x + normals[1] * w.y + normals[2] * w.z)
                .try_normalize()
                .unwrap_or(face_normal);
            texels.push(SurfaceTexel {
                x,
                y,
                position,
                normal,
            });
        }
    }
    texels
}

/// Vertex weights of the triangle's point nearest `point` in UV space, or `None` for a triangle
/// without UV area.
fn nearest_weights(uvs: [Vec2; 3], point: Vec2) -> Option<Vec3> {
    let lift = |uv: Vec2| uv.extend(0.0);
    let [a, b, c] = uvs.map(lift);
    let nearest = closest_point_on_triangle(lift(point), a, b, c).truncate();
    let area = (b - a).cross(c - a).z;
    if area == 0.0 || !area.is_finite() {
        return None;
    }
    let weight = |p: Vec2, q: Vec2| (lift(p) - lift(nearest)).cross(lift(q) - lift(nearest)).z;
    let w = Vec3::new(
        weight(uvs[1], uvs[2]),
        weight(uvs[2], uvs[0]),
        weight(uvs[0], uvs[1]),
    ) / area;
    let w = w.max(Vec3::ZERO);
    Some(w / w.element_sum().max(f32::MIN_POSITIVE))
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
    fn thin_triangles_touch_texels_whose_centers_they_miss() {
        // A sliver in texel column 1 of a 4² map, 0.3 texels wide, left of the column's centers.
        let uvs = [
            Vec2::new(0.30, 0.05),
            Vec2::new(0.375, 0.05),
            Vec2::new(0.30, 0.95),
        ];
        let vertices = uvs.map(|uv| Vec3::new(uv.x, 0.0, uv.y));
        let normals = [Vec3::Y; 3];
        assert!(triangle_texels(vertices, normals, uvs, Vec3::Y, 4).is_empty());
        let touched = touched_texels(vertices, normals, uvs, Vec3::Y, 4, |_, _| false);
        let cells: Vec<(u32, u32)> = touched.iter().map(|t| (t.x, t.y)).collect();
        assert_eq!(cells, [(1, 0), (1, 1), (1, 2), (1, 3)]);
        for texel in &touched {
            let uv = Vec2::new(texel.position.x, texel.position.z);
            assert!(
                uv.x >= 0.30 - 1e-6 && uv.x <= 0.375 + 1e-6,
                "{uv} lies on the sliver"
            );
            assert!(
                (uv.y - (texel.y as f32 + 0.5) / 4.0).abs() < 0.13,
                "{uv} near the center row"
            );
        }
        let owned = touched_texels(vertices, normals, uvs, Vec3::Y, 4, |_, y| y == 2);
        assert_eq!(owned.len(), 3, "owned texels are skipped");
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
