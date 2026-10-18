//! What a light's baked shadows were traced for beyond its position and radius: nothing for
//! point-like lights, the shape of an area light, the direction and disk size of a sun, or the
//! open sky. A baked light whose shape no longer matches is lit as realtime, like a light that
//! moved.

use bevy::math::{Quat, Vec3};

use super::area::AreaLightShape;
use super::component::DynamicLight;
use super::kinds::LightType;

/// `1 − cos θ` for the largest turn θ (0.01 rad) of a sun's direction that still matches a bake.
const SUN_DIRECTION_TOLERANCE: f32 = 5.0e-5;
/// The largest change of a sun's angular diameter in degrees that still matches a bake.
const SUN_DIAMETER_TOLERANCE: f32 = 1.0e-3;

/// The shape a baked light's shadows were traced for.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LightShape {
    /// Point-like lights, shadowed from their position. They may turn freely.
    #[default]
    Point,
    /// Rectangle and tube lights, shadowed over their shape.
    Area(AreaLightShape),
    /// A sun, shadowed towards its disk at infinity.
    Sun(SunDisk),
    /// The sky, shadowed by how much of the sky a surface sees. That doesn't depend on the sky's
    /// image or turn, so the sky may turn and change its image freely.
    Sky,
}

impl LightShape {
    /// The shape of `light` turned by `rotation`.
    pub fn of(light: &DynamicLight, rotation: Quat) -> Self {
        if let Some(area) = AreaLightShape::of(light, rotation) {
            return Self::Area(area);
        }
        if light.light_type == LightType::Sky {
            return Self::Sky;
        }
        SunDisk::of(light, rotation).map_or(Self::Point, Self::Sun)
    }

    pub fn area(&self) -> Option<AreaLightShape> {
        match self {
            Self::Area(area) => Some(*area),
            _ => None,
        }
    }

    pub fn sun(&self) -> Option<SunDisk> {
        match self {
            Self::Sun(sun) => Some(*sun),
            _ => None,
        }
    }

    /// Light from infinitely far away, which has no position or radius (see
    /// [`LightType::is_distant`]).
    pub fn is_distant(&self) -> bool {
        matches!(self, Self::Sun(_) | Self::Sky)
    }

    /// Whether a light shaped like `other` can still use shadows baked for this shape.
    pub fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Point, Self::Point) => true,
            (Self::Area(baked), Self::Area(now)) => baked.matches(now),
            (Self::Sun(baked), Self::Sun(now)) => baked.matches(now),
            (Self::Sky, Self::Sky) => true,
            _ => false,
        }
    }
}

/// What a sun's baked shadows were traced for: its orientation and the size of its disk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunDisk {
    /// The light's rotation; the sun shines along its forward axis (−Z).
    pub rotation: Quat,
    /// In degrees, as on [`DynamicLight::angular_diameter`].
    pub angular_diameter: f32,
}

impl SunDisk {
    /// The disk of a sun turned by `rotation`; `None` for every other type.
    pub fn of(light: &DynamicLight, rotation: Quat) -> Option<Self> {
        (light.light_type == LightType::Sun).then_some(Self {
            rotation,
            angular_diameter: light.angular_diameter,
        })
    }

    /// Unit vector from the scene towards the sun: the light's back axis.
    pub fn toward(&self) -> Vec3 {
        (self.rotation * Vec3::Z).normalize()
    }

    /// Half the angular diameter, in radians.
    pub fn angular_radius(&self) -> f32 {
        (self.angular_diameter * 0.5).to_radians()
    }

    /// Whether a sun like `other` can still use shadows baked for this one. Only the direction
    /// and the disk size count: turning a sun around its own axis changes nothing it lights.
    pub fn matches(&self, other: &Self) -> bool {
        self.toward().dot(other.toward()) >= 1.0 - SUN_DIRECTION_TOLERANCE
            && (self.angular_diameter - other.angular_diameter).abs() <= SUN_DIAMETER_TOLERANCE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suns_match_by_direction_and_disk_only() {
        let light = DynamicLight::sun();
        let down = Quat::from_rotation_x(-1.2);
        let baked = SunDisk::of(&light, down).unwrap();
        let spun = SunDisk::of(&light, Quat::from_axis_angle(baked.toward(), 1.0) * down).unwrap();
        assert!(
            baked.matches(&spun),
            "spinning around the sun's axis keeps the bake"
        );
        let turned = SunDisk::of(&light, Quat::from_rotation_x(0.02) * down).unwrap();
        assert!(!baked.matches(&turned), "turned");
        let wider = DynamicLight {
            angular_diameter: 2.0,
            ..light.clone()
        };
        assert!(!baked.matches(&SunDisk::of(&wider, down).unwrap()), "wider");
    }

    #[test]
    fn shapes_only_match_their_own_kind() {
        let rotation = Quat::from_rotation_x(-0.5);
        let sun = LightShape::of(&DynamicLight::sun(), rotation);
        let panel = LightShape::of(&DynamicLight::rectangle(1.0, 1.0), rotation);
        let point = LightShape::of(&DynamicLight::point(), rotation);
        let sky = LightShape::of(&DynamicLight::sky(), rotation);
        assert!(sun.is_distant() && sun.sun().is_some());
        assert!(panel.area().is_some() && !panel.is_distant());
        assert_eq!(point, LightShape::Point);
        assert!(sky.is_distant() && sky.sun().is_none());
        assert!(sun.matches(&sun) && panel.matches(&panel) && point.matches(&point));
        assert!(!sun.matches(&point) && !point.matches(&panel) && !panel.matches(&sun));
        assert!(!sky.matches(&sun) && !sun.matches(&sky));
        let turned = LightShape::of(&DynamicLight::sky(), Quat::from_rotation_y(2.0));
        assert!(sky.matches(&turned), "the sky may turn");
    }

    #[test]
    fn the_sun_shines_along_its_forward_axis() {
        let rotation = Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2);
        let sun = SunDisk::of(&DynamicLight::sun(), rotation).unwrap();
        assert!(
            sun.toward().abs_diff_eq(Vec3::Y, 1e-6),
            "a sun shining down is above"
        );
        assert!((sun.angular_radius() - 0.265_f32.to_radians()).abs() < 1e-7);
    }
}
