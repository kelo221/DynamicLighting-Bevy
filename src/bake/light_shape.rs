//! What a baked light's shape means to the tracer: its reach, shadow precision and facing test.

use bevy::math::Vec3;

use crate::light::LightShape;
use crate::light::area::AreaShape;

use super::area::AREA_SHADOW_BITS;
use super::input::BakeLight;
use super::mesh_bake::NDOTL_TOLERANCE;

impl BakeLight {
    /// The world-space shape of an area light.
    pub fn area_shape(&self) -> Option<AreaShape> {
        self.shape.area().map(|area| area.at(self.position))
    }

    /// How far the light reaches past its shape: the radius minus half the shape's diagonal, as
    /// at runtime, so every lit texel lies within `radius` of the position. The radius itself
    /// for point-like lights.
    pub fn shape_reach(&self) -> f32 {
        self.radius
            - self
                .shape
                .area()
                .map_or(0.0, |area| area.half_extents().length())
    }

    /// Bits per texel of the light's baked shadows: 1 for hard shadows (point-like lights and
    /// suns without a disk), [`AREA_SHADOW_BITS`] for soft ones.
    pub fn shadow_bits_per_texel(&self) -> u32 {
        match self.shape {
            LightShape::Point => 1,
            LightShape::Area(_) => AREA_SHADOW_BITS,
            LightShape::Sun(sun) if sun.angular_diameter > 0.0 => AREA_SHADOW_BITS,
            LightShape::Sun(_) => 1,
        }
    }

    /// Henry's facing test for a flat triangle with unit face `normal`: whether the light may be
    /// in front of it, seen from its center. An area light passes when any corner of its shape
    /// does; a rectangle also needs a vertex of the triangle on its lit side. A sun passes when
    /// any part of its disk does.
    pub fn may_face(&self, vertices: [Vec3; 3], normal: Vec3) -> bool {
        let center = (vertices[0] + vertices[1] + vertices[2]) / 3.0;
        let faces =
            |point: Vec3| normal.dot((point - center).normalize_or_zero()) > NDOTL_TOLERANCE;
        if let Some(sun) = self.shape.sun() {
            return normal.dot(sun.toward()) + sun.angular_radius().sin() > NDOTL_TOLERANCE;
        }
        let Some(shape) = self.area_shape() else {
            return faces(self.position);
        };
        let lit_side = !shape.rectangle
            || vertices
                .iter()
                .any(|&vertex| shape.forward.dot(vertex - shape.center) > 0.0);
        lit_side && shape.corners().into_iter().any(faces)
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::{Quat, Vec2};

    use crate::bake::tests::{point_light, sun_light};
    use crate::light::{AreaLightShape, LightType};

    use super::*;

    #[test]
    fn area_lights_reach_and_face_with_their_shape() {
        let mut light = point_light(1, Vec3::Y * 2.0, 3.0);
        assert_eq!(
            (light.shape_reach(), light.shadow_bits_per_texel()),
            (3.0, 1)
        );
        light.radius = 5.0;
        light.shape = LightShape::Area(AreaLightShape {
            kind: LightType::Tube,
            rotation: Quat::IDENTITY,
            size: Vec2::new(4.0, 0.0),
        });
        assert_eq!(
            (light.shape_reach(), light.shadow_bits_per_texel()),
            (3.0, 4)
        );
        // A wall facing +X at x = 1.5 still sees the tube's far end.
        let wall = [
            Vec3::new(1.5, 0.0, 0.0),
            Vec3::new(1.5, 1.0, 0.0),
            Vec3::new(1.5, 0.0, 1.0),
        ];
        assert!(light.may_face(wall, Vec3::X));
        let far_wall = wall.map(|v| v + Vec3::X * 1.0);
        assert!(!light.may_face(far_wall, Vec3::X));
        // A floor above a downward panel is on its dark side.
        light.shape = LightShape::Area(AreaLightShape {
            kind: LightType::Rectangle,
            rotation: Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2),
            size: Vec2::ONE,
        });
        let floor = [Vec3::ZERO, Vec3::Z, Vec3::X].map(|v| v + Vec3::Y * 3.0);
        assert!(!light.may_face(floor, Vec3::Y));
        assert!(light.may_face(floor.map(|v| v - Vec3::Y * 3.0), Vec3::Y));
    }

    #[test]
    fn suns_face_by_direction_and_soften_with_their_disk() {
        let floor = [Vec3::ZERO, Vec3::Z, Vec3::X];
        let low = sun_light(1, Vec3::new(1.0, 0.02, 0.0), 0.0);
        assert_eq!(low.shadow_bits_per_texel(), 1, "no disk, hard shadows");
        assert!(low.may_face(floor, Vec3::Y));
        assert!(low.may_face(floor, Vec3::NEG_Y), "within Henry's tolerance");
        assert!(!low.may_face(floor, Vec3::NEG_X));
        // A sun just below the tolerance still lights the floor with the top of its disk.
        let below = Vec3::new(1.0, -0.11, 0.0).normalize();
        assert!(!sun_light(2, below, 0.0).may_face(floor, Vec3::Y));
        let wide = sun_light(2, below, 10.0);
        assert!(wide.may_face(floor, Vec3::Y));
        assert_eq!(wide.shadow_bits_per_texel(), AREA_SHADOW_BITS);
    }
}
