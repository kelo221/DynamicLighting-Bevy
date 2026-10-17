//! The cell grid over the view of the camera that chose the realtime lights: screen tiles
//! across and down, depth slices out to the farthest realtime light. [`Grid::cell_at`] is the
//! CPU mirror of `realtime_span` in `shaders/clusters.wesl`.
//!
//! The shader finds a fragment's cell from the stored camera matrix, not from the view drawing
//! it, so the cells are volumes of the world that every camera can look up. A fragment outside
//! them (seen by another camera) visits every realtime light.

use bevy::math::{Mat4, UVec2, UVec3, Vec2, Vec3, Vec4, Vec4Swizzles};

use super::gpu::GpuClusterHeader;
use crate::light::RealtimeViewer;

/// Cells across, down and deep.
pub const GRID: UVec3 = UVec3::new(16, 9, 24);

/// The first depth slice ends here, or at half the far depth when that is nearer. It holds the
/// few fragments right in front of the camera, so the exponential slices start further out.
pub const FIRST_SLICE_DEPTH: f32 = 1.0;

/// Bounds are widened by this fraction of a cell before they are rounded to cells, so a fragment
/// on a cell border that the GPU rounds into the neighboring cell still finds its lights.
const CELL_PADDING: f32 = 1.0e-3;

/// A light's reach: nothing outside the sphere is lit by it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    pub center: Vec3,
    pub radius: f32,
}

/// The view-space bounding box of a cell, widened by `CELL_PADDING` of its size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellBox {
    pub min: Vec3,
    pub max: Vec3,
}

impl CellBox {
    /// Whether the view-space sphere at `center` with `radius` may reach the cell.
    pub fn reaches(&self, center: Vec3, radius: f32) -> bool {
        center.clamp(self.min, self.max).distance_squared(center) <= radius * radius
    }
}

/// Where a world position falls in the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// Outside the clustering camera's view: every realtime light may reach it.
    Outside,
    /// In view but past every realtime light (or, for orthographic cameras, before them).
    Empty,
    /// Inside the cell with this index: `(slice * GRID.y + y) * GRID.x + x`.
    At(u32),
}

/// One frame's grid.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    view_from_world: Mat4,
    clip_from_view: Mat4,
    clip_from_world: Mat4,
    depth_row: Vec4,
    perspective: bool,
    first_depth: f32,
    far: f32,
    slice_scale: f32,
    slice_bias: f32,
}

impl Grid {
    /// The grid that holds `spheres`, or `None` when none of them is in front of the camera.
    pub fn new(viewer: RealtimeViewer, spheres: impl IntoIterator<Item = Sphere>) -> Option<Self> {
        let depth_row = -viewer.view_from_world.row(2);
        let depth = |point: Vec3| depth_row.xyz().dot(point) + depth_row.w;
        let (mut nearest, mut far) = (f32::INFINITY, f32::NEG_INFINITY);
        for sphere in spheres {
            let center = depth(sphere.center);
            nearest = nearest.min(center - sphere.radius);
            far = far.max(center + sphere.radius);
        }
        let perspective = viewer.clip_from_view.w_axis.w == 0.0;
        let slices = GRID.z as f32;
        let (first_depth, slice_scale, slice_bias) = if perspective {
            let first = FIRST_SLICE_DEPTH.min(far * 0.5);
            if first <= 0.0 || first.is_nan() {
                return None;
            }
            let scale = (slices - 1.0) / (far / first).log2();
            (first, scale, -first.log2() * scale)
        } else {
            if far <= nearest || far.is_nan() || nearest.is_nan() {
                return None;
            }
            let scale = slices / (far - nearest);
            (0.0, scale, -nearest * scale)
        };
        Some(Self {
            view_from_world: viewer.view_from_world,
            clip_from_view: viewer.clip_from_view,
            clip_from_world: viewer.clip_from_view * viewer.view_from_world,
            depth_row,
            perspective,
            first_depth,
            far,
            slice_scale,
            slice_bias,
        })
    }

    /// The buffer header for this grid, with the light indices starting at word `entry_base`.
    pub fn header(&self, entry_base: u32) -> GpuClusterHeader {
        GpuClusterHeader {
            clip_from_world: self.clip_from_world.to_cols_array_2d(),
            depth_row: self.depth_row.to_array(),
            dims: GRID.to_array(),
            entry_base,
            first_depth: self.first_depth,
            far: self.far,
            slice_scale: self.slice_scale,
            slice_bias: self.slice_bias,
            flags: if self.perspective {
                GpuClusterHeader::FLAG_PERSPECTIVE
            } else {
                0
            },
            _padding: [0; 3],
        }
    }

    /// A world position in view space, where [`Self::cell_boxes`] are.
    pub fn view_point(&self, world: Vec3) -> Vec3 {
        self.view_from_world.transform_point3(world)
    }

    /// Every cell's view-space bounding box, indexed like [`Cell::At`]. A cell is the convex
    /// piece of its screen tile between two slice depths, so its corners bound it.
    pub fn cell_boxes(&self) -> Vec<CellBox> {
        let view_from_clip = self.clip_from_view.inverse();
        let tiles = GRID.truncate();
        // Each tile corner in view space: a point on its ray (perspective) or its x and y.
        let mut corners = Vec::with_capacity(((tiles.x + 1) * (tiles.y + 1)) as usize);
        for y in 0..=tiles.y {
            for x in 0..=tiles.x {
                let ndc = UVec2::new(x, y).as_vec2() / tiles.as_vec2() * 2.0 - 1.0;
                corners.push(view_from_clip.project_point3(ndc.extend(0.5)));
            }
        }
        let at_depth = |corner: Vec3, depth: f32| {
            if self.perspective {
                corner * (depth / -corner.z)
            } else {
                corner.with_z(-depth)
            }
        };
        let mut boxes = Vec::with_capacity(GRID.element_product() as usize);
        for z in 0..GRID.z {
            let depths = [self.slice_start(z), self.slice_start(z + 1)];
            for y in 0..tiles.y {
                for x in 0..tiles.x {
                    let (mut min, mut max) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let corner = corners[((y + dy) * (tiles.x + 1) + x + dx) as usize];
                        for depth in depths {
                            let point = at_depth(corner, depth);
                            min = min.min(point);
                            max = max.max(point);
                        }
                    }
                    let padding = (max - min) * CELL_PADDING;
                    boxes.push(CellBox {
                        min: min - padding,
                        max: max + padding,
                    });
                }
            }
        }
        boxes
    }

    /// The view depth where depth slice `slice` starts; `GRID.z` gives where the last one ends.
    fn slice_start(&self, slice: u32) -> f32 {
        if slice == GRID.z {
            self.far
        } else if !self.perspective {
            (slice as f32 - self.slice_bias) / self.slice_scale
        } else if slice == 0 {
            0.0
        } else {
            ((slice as f32 - 1.0 - self.slice_bias) / self.slice_scale).exp2()
        }
    }

    /// The depth slice coordinate of a view depth; negative before the grid.
    fn slice_coordinate(&self, depth: f32) -> f32 {
        if !self.perspective {
            depth * self.slice_scale + self.slice_bias
        } else if depth < self.first_depth {
            0.0
        } else {
            1.0 + depth.log2() * self.slice_scale + self.slice_bias
        }
    }

    /// The cell of a world position, computed the way the shader does.
    pub fn cell_at(&self, world: Vec3) -> Cell {
        let clip = self.clip_from_world * world.extend(1.0);
        if clip.w <= 0.0 {
            return Cell::Outside;
        }
        let ndc = clip.xy() / clip.w;
        if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 {
            return Cell::Outside;
        }
        let depth = self.depth_row.xyz().dot(world) + self.depth_row.w;
        let slice = self.slice_coordinate(depth);
        if depth > self.far || slice < 0.0 {
            return Cell::Empty;
        }
        let tile = ((ndc * 0.5 + 0.5) * GRID.truncate().as_vec2()).as_uvec2();
        let x = tile.x.min(GRID.x - 1);
        let y = tile.y.min(GRID.y - 1);
        let z = (slice as u32).min(GRID.z - 1);
        Cell::At((z * GRID.y + y) * GRID.x + x)
    }

    /// The first and last cell (inclusive) in each direction that `sphere` may reach, or `None`
    /// when it is out of view. Conservative: every point of the sphere inside the view falls in
    /// one of these cells.
    pub fn cells_touched(&self, sphere: Sphere) -> Option<(UVec3, UVec3)> {
        let center = self.view_from_world.transform_point3(sphere.center);
        let depth = -center.z;
        if self.perspective && depth + sphere.radius <= 0.0 {
            return None;
        }
        let (low, high) = self.screen_bounds(center, sphere.radius)?;
        let to_cells = |ndc: Vec2| (ndc * 0.5 + 0.5) * GRID.truncate().as_vec2();
        let (low, high) = (to_cells(low), to_cells(high));
        let near = self.slice_coordinate(depth - sphere.radius);
        let far = self.slice_coordinate(depth + sphere.radius);
        let first = |value: f32, count: u32| cell_index(value - CELL_PADDING, count);
        let last = |value: f32, count: u32| cell_index(value + CELL_PADDING, count);
        Some((
            UVec3::new(
                first(low.x, GRID.x),
                first(low.y, GRID.y),
                first(near, GRID.z),
            ),
            UVec3::new(
                last(high.x, GRID.x),
                last(high.y, GRID.y),
                last(far, GRID.z),
            ),
        ))
    }

    /// The normalized device bounds of the view-space box around a sphere, or `None` when the
    /// box is beside the view. The whole screen when the box reaches behind a perspective camera.
    fn screen_bounds(&self, center: Vec3, radius: f32) -> Option<(Vec2, Vec2)> {
        let (mut low, mut high) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for corner in 0..8 {
            let sign = |bit: u32| if corner & bit == 0 { -1.0 } else { 1.0 };
            let point = center + radius * Vec3::new(sign(1), sign(2), sign(4));
            let clip = self.clip_from_view * point.extend(1.0);
            if clip.w <= 0.0 {
                return Some((Vec2::NEG_ONE, Vec2::ONE));
            }
            let ndc = clip.xy() / clip.w;
            low = low.min(ndc);
            high = high.max(ndc);
        }
        let beside = high.x < -1.0 || high.y < -1.0 || low.x > 1.0 || low.y > 1.0;
        (!beside).then_some((low.max(Vec2::NEG_ONE), high.min(Vec2::ONE)))
    }
}

/// The cell holding coordinate `value` along an axis with `count` cells, clamped to the grid.
fn cell_index(value: f32, count: u32) -> u32 {
    (value.max(0.0) as u32).min(count - 1)
}
