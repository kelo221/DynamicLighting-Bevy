//! Starting points for common lights, from Henry's "Light" creation menu.

use bevy::prelude::*;

use super::component::DynamicLight;
use super::kinds::LightType;

impl DynamicLight {
    /// A point light with the default color, intensity and 4 m radius.
    pub fn point() -> Self {
        Self::default()
    }

    /// A spot light shining along the entity's forward axis (`Transform::forward`), fading from
    /// 26° to 30° off the axis.
    pub fn spot() -> Self {
        Self {
            light_type: LightType::Spot,
            ..default()
        }
    }

    /// A discoball: small spot cones, 12.5° wide with a 1.5° fade, scattered around the light.
    pub fn discoball() -> Self {
        Self {
            light_type: LightType::Discoball,
            inner_cutoff: 12.5,
            outer_cutoff: 14.0,
            ..default()
        }
    }

    /// A `width` by `height` light panel in the entity's local XY plane, lighting along its
    /// forward axis (`Transform::forward`): point it down for a ceiling panel.
    pub fn rectangle(width: f32, height: f32) -> Self {
        Self {
            light_type: LightType::Rectangle,
            size: Vec2::new(width, height),
            ..default()
        }
    }

    /// A tube light `length` long along the entity's local X axis, like a fluorescent tube.
    pub fn tube(length: f32) -> Self {
        Self {
            light_type: LightType::Tube,
            size: Vec2::new(length, 0.0),
            ..default()
        }
    }

    /// A sun shining along the entity's forward axis (`Transform::forward`), with Henry's sun
    /// intensity of 2 and the real sun's 0.53° disk, which gives slightly soft baked shadows.
    pub fn sun() -> Self {
        Self {
            light_type: LightType::Sun,
            ..default()
        }
    }

    /// The sky around the scene at intensity 1, which shows its image as it is. Add
    /// `environment::SkyEnvironment` with the image next to it; turning the entity turns the sky.
    pub fn sky() -> Self {
        Self {
            light_type: LightType::Sky,
            intensity: 1.0,
            ..default()
        }
    }
}

/// A [`DynamicLight::sun`] whose light travels along `direction`. Henry's editor menu makes a
/// point light 2.5 km away instead, because his package has no directional lights. Spawn the
/// returned bundle; the position only places the gizmo.
pub fn sun_light(direction: Dir3) -> (DynamicLight, Transform) {
    (
        DynamicLight::sun(),
        Transform::default().looking_to(direction, perpendicular(direction)),
    )
}

/// An up vector that isn't parallel to `direction`.
fn perpendicular(direction: Dir3) -> Dir3 {
    if direction.y.abs() > 0.99 {
        Dir3::Z
    } else {
        Dir3::Y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_valid_lights() {
        for light in [
            DynamicLight::point(),
            DynamicLight::spot(),
            DynamicLight::discoball(),
            DynamicLight::rectangle(1.0, 0.5),
            DynamicLight::tube(1.2),
            DynamicLight::sun(),
        ] {
            assert!(light.is_valid(), "{light:?}");
        }
        assert_eq!(DynamicLight::spot().light_type, LightType::Spot);
        let disco = DynamicLight::discoball();
        assert_eq!((disco.inner_cutoff, disco.outer_cutoff), (12.5, 14.0));
    }

    #[test]
    fn the_sun_shines_along_its_direction() {
        let direction = Dir3::new(Vec3::new(1.0, -2.0, 0.5)).unwrap();
        let (light, transform) = sun_light(direction);
        assert_eq!(light.light_type, LightType::Sun);
        assert!(transform.forward().dot(direction.as_vec3()) > 0.9999);
        let (_, straight_down) = sun_light(Dir3::NEG_Y);
        assert!(straight_down.forward().dot(Vec3::NEG_Y) > 0.9999);
        assert!(straight_down.rotation.is_finite());
    }
}
