//! Area lights bake their shadows from points spread over their shape. Each point is baked
//! exactly like a point light (1-bit visibility from one fixed position) and lights the scene
//! with an equal share of the area light, so shadow edges soften across the shape while every
//! baked light keeps to the rules of the original package.
//!
//! A panel's points form a lattice in which every point has a column and a row of its own:
//! point `i` of `n` sits in column `i` and row `i · g mod n`. Shadow edges running either way
//! across the panel then soften in `n` steps, not just those across its long side. A tube is
//! the same lattice with no height: a row of points along its length.

use bevy::math::{Quat, Vec2, Vec3};

use super::component::DynamicLight;
use crate::shared::BakeId;

/// The most points an area light is shadowed from.
pub const MAX_SHADOW_POINTS: u32 = 16;

/// One point a light's baked shadows are traced from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowPoint {
    /// Which of the light's points this is: its lattice column.
    pub index: u32,
    /// The point's offset from the light's position along the light's local X and Y.
    pub offset: Vec2,
    /// Where the point is, in world space.
    pub position: Vec3,
    /// How far the point's baked light reaches: the light's radius plus the point's distance
    /// from the light's position, so it covers everything the shape lights.
    pub radius: f32,
}

impl DynamicLight {
    /// How many points the light's baked shadows come from: [`Self::shadow_points`] (1 to
    /// [`MAX_SHADOW_POINTS`]) for rectangle and tube lights, otherwise 1.
    pub fn shadow_point_count(&self) -> u32 {
        if self.light_type.is_area() {
            self.shadow_points.clamp(1, MAX_SHADOW_POINTS)
        } else {
            1
        }
    }

    /// The points the light is shadowed from when it stands at `position` turned by
    /// `rotation`: a lattice over its shape, or just `position`.
    pub fn shadow_point_layout(
        &self,
        position: Vec3,
        rotation: Quat,
    ) -> impl Iterator<Item = ShadowPoint> + use<> {
        let count = self.shadow_point_count();
        let half = self.half_extents();
        let step = lattice_step(count, half);
        let radius = self.radius;
        (0..count).map(move |index| {
            let offset = half * lattice_cell(index, step, count);
            ShadowPoint {
                index,
                offset,
                position: position + rotation * offset.extend(0.0),
                radius: radius + offset.length(),
            }
        })
    }
}

/// Where point `index` of `count` sits on a shape spanning `[-1, 1]²`: the center of column
/// `index` and of row `index · step mod count`.
fn lattice_cell(index: u32, step: u32, count: u32) -> Vec2 {
    let row = (index * step) % count;
    let center = |cell: u32| (2 * cell + 1) as f32 / count as f32 - 1.0;
    Vec2::new(center(index), center(row))
}

/// The row step of the lattice of `count` points on a shape with half extents `half`: of the
/// steps that give every point a row of its own (coprime with `count`), the one that keeps the
/// points farthest apart. Ties go to the step nearest `count / φ`, which spreads the points
/// like a Fibonacci lattice, then to the smaller step.
fn lattice_step(count: u32, half: Vec2) -> u32 {
    let golden = count as f32 / 1.618_034;
    let mut best = (1, 0.0, f32::MAX);
    for step in (1..count).filter(|&step| coprime(step, count)) {
        let spacing = closest_pair(count, step, half);
        let detour = (step as f32 - golden).abs();
        let better = spacing > best.1 * (1.0 + 1.0e-5)
            || (spacing >= best.1 * (1.0 - 1.0e-5) && detour < best.2 - 1.0e-5);
        if better {
            best = (step, spacing, detour);
        }
    }
    best.0
}

/// The distance between the two closest points of a lattice, in the shape's units.
fn closest_pair(count: u32, step: u32, half: Vec2) -> f32 {
    let point = |index| half * lattice_cell(index, step, count);
    let mut closest = f32::MAX;
    for a in 0..count {
        for b in a + 1..count {
            closest = closest.min(point(a).distance(point(b)));
        }
    }
    closest
}

fn coprime(mut a: u32, mut b: u32) -> bool {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a == 1
}

/// The bake id of shadow point `index` of a light baked as `count` points. A light with one
/// point keeps its own id, so it bakes exactly like a point light.
pub fn shadow_point_id(light: BakeId, index: u32, count: u32) -> BakeId {
    if count == 1 { light } else { light.part(index) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(light: &DynamicLight, rotation: Quat) -> Vec<ShadowPoint> {
        light.shadow_point_layout(Vec3::Y, rotation).collect()
    }

    fn panel(width: f32, height: f32, points: u32) -> DynamicLight {
        DynamicLight {
            shadow_points: points,
            ..DynamicLight::rectangle(width, height)
        }
    }

    #[test]
    fn point_like_lights_have_one_point_at_their_position() {
        for light in [DynamicLight::point(), DynamicLight::spot()] {
            let points = layout(&light, Quat::from_rotation_z(0.7));
            assert_eq!(points.len(), 1);
            assert_eq!(points[0].position, Vec3::Y);
            assert_eq!(points[0].radius, light.radius);
            assert_eq!(shadow_point_id(BakeId(9), 0, 1), BakeId(9));
        }
        let single = DynamicLight {
            shadow_points: 1,
            ..DynamicLight::tube(2.0)
        };
        assert_eq!(layout(&single, Quat::IDENTITY)[0].position, Vec3::Y);
        assert_eq!(
            layout(&panel(2.0, 2.0, 1), Quat::IDENTITY)[0].position,
            Vec3::Y
        );
    }

    #[test]
    fn tube_points_split_the_length_evenly() {
        let rotation = Quat::from_rotation_y(0.5);
        let tube = DynamicLight::tube(3.0);
        let points = layout(&tube, rotation);
        for (point, expected) in points.iter().zip([-1.0, 0.0, 1.0]) {
            assert!((point.offset.x - expected).abs() < 1e-6, "{point:?}");
            assert_eq!(point.offset.y, 0.0);
        }
        let axis = rotation * Vec3::X;
        assert!(points[2].position.distance(Vec3::Y + axis) < 1e-6);
        assert!((points[0].radius - (tube.radius + 1.0)).abs() < 1e-6);
    }

    #[test]
    fn every_panel_point_has_a_column_and_a_row_of_its_own() {
        for count in 2..=MAX_SHADOW_POINTS {
            for (width, height) in [(1.0, 1.0), (1.6, 0.4), (0.4, 1.6), (3.0, 0.1)] {
                let light = panel(width, height, count);
                let points = layout(&light, Quat::IDENTITY);
                let half = light.half_extents();
                let mut columns: Vec<u32> = Vec::new();
                let mut rows: Vec<u32> = Vec::new();
                for point in &points {
                    let cell = (point.offset / half + 1.0) * count as f32 / 2.0 - 0.5;
                    assert!(cell.distance(cell.round()) < 1e-3, "{cell}");
                    columns.push(cell.x.round() as u32);
                    rows.push(cell.y.round() as u32);
                }
                columns.sort_unstable();
                rows.sort_unstable();
                let all: Vec<u32> = (0..count).collect();
                assert_eq!(columns, all, "{count} {width}×{height}");
                assert_eq!(rows, all, "{count} {width}×{height}");
                // The points average to the light's position, so the shadow isn't shifted.
                let mean = points.iter().map(|p| p.position).sum::<Vec3>() / count as f32;
                assert!(mean.distance(Vec3::Y) < 1e-5, "{mean}");
            }
        }
    }

    #[test]
    fn square_panels_avoid_the_diagonal() {
        // Points on a diagonal would leave edges along the other diagonal hard.
        for count in 3..=MAX_SHADOW_POINTS {
            let points = layout(&panel(1.0, 1.0, count), Quat::IDENTITY);
            let on_diagonal = points
                .iter()
                .filter(|p| (p.offset.x - p.offset.y).abs() < 1e-5)
                .count();
            assert!(on_diagonal < count as usize, "{count}");
        }
        // Three points: rows 0, 2, 1 rather than 0, 1, 2.
        let three = layout(&panel(1.0, 1.0, 3), Quat::IDENTITY);
        let rows: Vec<f32> = three.iter().map(|p| p.offset.y).collect();
        assert!((rows[1] - 1.0 / 3.0).abs() < 1e-6, "{rows:?}");
    }

    #[test]
    fn layouts_are_stable() {
        let light = panel(1.6, 0.4, 16);
        let rotation = Quat::from_rotation_x(-1.0);
        assert_eq!(layout(&light, rotation), layout(&light, rotation));
    }

    #[test]
    fn counts_are_clamped_and_ids_are_distinct() {
        let mut tube = DynamicLight::tube(1.0);
        tube.shadow_points = 0;
        assert_eq!(tube.shadow_point_count(), 1);
        tube.shadow_points = 40;
        assert_eq!(tube.shadow_point_count(), MAX_SHADOW_POINTS);
        let ids: Vec<BakeId> = (0..3).map(|i| shadow_point_id(BakeId(5), i, 3)).collect();
        assert!(ids[0] != ids[1] && ids[1] != ids[2] && !ids.contains(&BakeId(5)));
    }

    #[test]
    fn every_point_reaches_all_the_shape_lights() {
        let light = DynamicLight {
            radius: 3.0,
            ..panel(1.6, 0.8, 7)
        };
        let rotation = Quat::from_rotation_x(-1.2);
        let reach = light.shape_reach();
        let half = light.half_extents();
        for point in light.shadow_point_layout(Vec3::Y, rotation) {
            // The far corner of the shape grown by the reach in every direction.
            for corner in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
                let local = Vec3::new(corner.0 * half.x, corner.1 * half.y, 0.0);
                let far = Vec3::Y + rotation * local;
                let distance = far.distance(point.position) + reach;
                assert!(
                    distance <= point.radius + 1e-5,
                    "{distance} > {}",
                    point.radius
                );
            }
        }
    }
}
