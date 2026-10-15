//! Area lights bake their shadows from a row of points along their long side. Each point is
//! baked exactly like a point light (1-bit visibility from one fixed position) and lights the
//! scene with an equal share of the area light, so shadow edges soften along the shape while
//! every baked light keeps to the rules of the original package.

use bevy::math::{Quat, Vec3};

use super::component::DynamicLight;
use crate::shared::BakeId;

/// The most points an area light is shadowed from.
pub const MAX_SHADOW_POINTS: u32 = 8;

/// One point a light's baked shadows are traced from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowPoint {
    /// Which of the light's points this is, counted from the negative end of the long axis.
    pub index: u32,
    /// The point's distance from the light's position along [`DynamicLight::long_axis`].
    pub offset: f32,
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

    /// The local axis an area light's shadow points spread along: X for tubes and for panels at
    /// least as wide as they are tall, otherwise Y.
    pub fn long_axis(&self) -> Vec3 {
        let half = self.half_extents();
        if half.x >= half.y { Vec3::X } else { Vec3::Y }
    }

    /// The points the light is shadowed from when it stands at `position` turned by
    /// `rotation`: the centers of equal parts of its long side, or just `position`.
    pub fn shadow_point_layout(
        &self,
        position: Vec3,
        rotation: Quat,
    ) -> impl Iterator<Item = ShadowPoint> + use<> {
        let count = self.shadow_point_count();
        let half = self.half_extents().max_element();
        let axis = rotation * self.long_axis();
        let radius = self.radius;
        (0..count).map(move |index| {
            let offset = half * ((2 * index + 1) as f32 / count as f32 - 1.0);
            ShadowPoint {
                index,
                offset,
                position: position + axis * offset,
                radius: radius + offset.abs(),
            }
        })
    }
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
    }

    #[test]
    fn points_split_the_long_side_evenly() {
        let rotation = Quat::from_rotation_y(0.5);
        let tube = DynamicLight::tube(3.0);
        let points = layout(&tube, rotation);
        for (point, expected) in points.iter().zip([-1.0, 0.0, 1.0]) {
            assert!((point.offset - expected).abs() < 1e-6, "{point:?}");
        }
        let axis = rotation * Vec3::X;
        assert!(points[2].position.distance(Vec3::Y + axis) < 1e-6);
        assert!((points[0].radius - (tube.radius + 1.0)).abs() < 1e-6);

        let tall = DynamicLight {
            shadow_points: 2,
            ..DynamicLight::rectangle(0.4, 1.6)
        };
        let points = layout(&tall, Quat::IDENTITY);
        assert_eq!(tall.long_axis(), Vec3::Y);
        assert_eq!(points[0].position, Vec3::new(0.0, 0.6, 0.0));
        assert_eq!(points[1].position, Vec3::new(0.0, 1.4, 0.0));
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
        let panel = DynamicLight {
            radius: 3.0,
            shadow_points: 4,
            ..DynamicLight::rectangle(1.6, 0.8)
        };
        let rotation = Quat::from_rotation_x(-1.2);
        let reach = panel.shape_reach();
        let half = panel.half_extents();
        for point in panel.shadow_point_layout(Vec3::Y, rotation) {
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
