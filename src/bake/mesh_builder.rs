//! Henry's `MeshBuilder`: per-mesh triangle access, surface area and shadow map size, plus the
//! texel rectangles each bake step works on.

use bevy::math::{Vec2, Vec3};

use super::bits::TexelRect;
use super::geometry::{lightmap_size, triangle_area};
use super::input::{BakeMesh, BakeOptions};

/// A bake mesh with its shadow map size decided and its UV1 sanitized.
pub struct PreparedMesh<'a> {
    pub mesh: &'a BakeMesh,
    pub resolution: u32,
    /// UV1 with non-finite coordinates replaced by zero, as Henry does.
    pub uv1: Vec<Vec2>,
    /// How many UV1 coordinates were non-finite.
    pub non_finite_uvs: usize,
}

impl<'a> PreparedMesh<'a> {
    pub fn new(mesh: &'a BakeMesh, options: &BakeOptions) -> Self {
        let surface_area: f32 = (0..mesh.triangle_count())
            .map(|i| {
                let [a, b, c] = triangle_vertices(mesh, i);
                triangle_area(a, b, c)
            })
            .sum();
        let mut non_finite_uvs = 0;
        let uv1 = mesh
            .uv1
            .iter()
            .map(|uv| {
                if uv.is_finite() {
                    *uv
                } else {
                    non_finite_uvs += 1;
                    Vec2::ZERO
                }
            })
            .collect();
        Self {
            mesh,
            resolution: lightmap_size(
                surface_area,
                options.pixel_density,
                options.max_lightmap_size,
            ),
            uv1,
            non_finite_uvs,
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.mesh.triangle_count()
    }

    pub fn vertices(&self, triangle: usize) -> [Vec3; 3] {
        triangle_vertices(self.mesh, triangle)
    }

    /// Vertex normals, or the face normal three times for meshes without normals.
    pub fn normals(&self, triangle: usize) -> [Vec3; 3] {
        if self.mesh.normals.is_empty() {
            let [a, b, c] = self.vertices(triangle);
            return [(b - a).cross(c - a).normalize_or_zero(); 3];
        }
        self.corners(triangle)
            .map(|corner| self.mesh.normals[corner as usize])
    }

    pub fn uvs(&self, triangle: usize) -> [Vec2; 3] {
        self.corners(triangle)
            .map(|corner| self.uv1[corner as usize])
    }

    fn corners(&self, triangle: usize) -> [u32; 3] {
        let corners = &self.mesh.indices[triangle * 3..][..3];
        [corners[0], corners[1], corners[2]]
    }
}

fn triangle_vertices(mesh: &BakeMesh, triangle: usize) -> [Vec3; 3] {
    let corners = &mesh.indices[triangle * 3..][..3];
    [0, 1, 2].map(|k| mesh.positions[corners[k] as usize])
}

/// UV bounds of a triangle as (min, max).
fn uv_bounds(uvs: [Vec2; 3]) -> (Vec2, Vec2) {
    (
        uvs[0].min(uvs[1]).min(uvs[2]),
        uvs[0].max(uvs[1]).max(uvs[2]),
    )
}

/// `value` clamped to the texel range `[0, size - 1]`.
fn clamp_texel(value: f32, size: u32) -> u32 {
    value.clamp(0.0, (size - 1) as f32) as u32
}

/// The texels whose shadow bits are stored for a triangle, before Henry's 2 px padding: the UV
/// bounds scaled to the map, floored/ceiled, grown by 2 texels and clamped to the map
/// (`DynamicTrianglesBuilder` constructor, `BuildShadows`).
pub fn crop_rect(uvs: [Vec2; 3], size: u32) -> TexelRect {
    let (min, max) = uv_bounds(uvs);
    let s = size as f32;
    TexelRect {
        min_x: clamp_texel((min.x * s).floor() - 2.0, size),
        min_y: clamp_texel((min.y * s).floor() - 2.0, size),
        max_x: clamp_texel((max.x * s).ceil() + 2.0, size),
        max_y: clamp_texel((max.y * s).ceil() + 2.0, size),
    }
}

/// The texels whose centers may lie inside the triangle (plus one texel of slack for rounding;
/// the barycentric test rejects the rest). Texel `x` has its center at `(x + 0.5) / size`.
pub fn trace_rect(uvs: [Vec2; 3], size: u32) -> TexelRect {
    grown_center_rect(uvs, size, 0.0)
}

/// The texels searched for any lit bit before a light is dropped from a triangle: the trace
/// rectangle grown by one texel, so tiny triangles keep the lights of their neighbours.
pub fn optimize_rect(uvs: [Vec2; 3], size: u32) -> TexelRect {
    grown_center_rect(uvs, size, 1.0)
}

fn grown_center_rect(uvs: [Vec2; 3], size: u32, grow: f32) -> TexelRect {
    let (min, max) = uv_bounds(uvs);
    let s = size as f32;
    TexelRect {
        min_x: clamp_texel((min.x * s - 0.5).floor() - grow, size),
        min_y: clamp_texel((min.y * s - 0.5).floor() - grow, size),
        max_x: clamp_texel((max.x * s - 0.5).ceil() + grow, size),
        max_y: clamp_texel((max.y * s - 0.5).ceil() + grow, size),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad_mesh(size: f32) -> BakeMesh {
        BakeMesh {
            positions: vec![
                Vec3::ZERO,
                Vec3::new(size, 0.0, 0.0),
                Vec3::new(size, 0.0, size),
                Vec3::new(0.0, 0.0, size),
            ],
            indices: vec![0, 2, 1, 0, 3, 2],
            uv1: vec![Vec2::ZERO, Vec2::X, Vec2::ONE, Vec2::new(f32::NAN, 1.0)],
            ..Default::default()
        }
    }

    #[test]
    fn size_area_and_uv_sanitizing() {
        let mesh = quad_mesh(2.0);
        let options = BakeOptions {
            pixel_density: 10.0,
            ..Default::default()
        };
        let prepared = PreparedMesh::new(&mesh, &options);
        assert_eq!(prepared.resolution, 20, "ceil(sqrt(10² · 4 m²))");
        assert_eq!(prepared.non_finite_uvs, 1);
        assert_eq!(prepared.uv1[3], Vec2::ZERO);
        // Flat normals face up for this winding (counter-clockwise seen from +Y).
        assert!(prepared.normals(0)[0].abs_diff_eq(Vec3::Y, 1e-6));
    }

    #[test]
    fn rects_follow_henrys_bounds_and_clamp_at_the_edges() {
        let uvs = [
            Vec2::new(0.25, 0.25),
            Vec2::new(0.5, 0.25),
            Vec2::new(0.25, 0.5),
        ];
        let crop = crop_rect(uvs, 16);
        assert_eq!(
            (crop.min_x, crop.min_y, crop.max_x, crop.max_y),
            (2, 2, 10, 10)
        );
        let trace = trace_rect(uvs, 16);
        assert_eq!((trace.min_x, trace.max_x), (3, 8));
        let optimize = optimize_rect(uvs, 16);
        assert_eq!((optimize.min_x, optimize.max_x), (2, 9));

        let corner = [Vec2::ZERO, Vec2::new(0.05, 0.0), Vec2::new(0.0, 0.05)];
        let crop = crop_rect(corner, 16);
        assert_eq!((crop.min_x, crop.min_y), (0, 0));
        let outside = [Vec2::splat(3.0), Vec2::splat(4.0), Vec2::new(3.0, 4.0)];
        let crop = crop_rect(outside, 16);
        assert_eq!((crop.min_x, crop.max_x), (15, 15));
    }
}
