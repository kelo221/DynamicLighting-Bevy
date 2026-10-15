//! Starting points for common lights, from Henry's "Light" creation menu.

use bevy::prelude::*;

use super::component::DynamicLight;
use super::kinds::LightType;

/// How far [`sun_light`] places its light from the world origin, in meters.
pub const SUN_DISTANCE: f32 = 2500.0;
/// The radius of [`sun_light`]'s light: far enough past [`SUN_DISTANCE`] to cover a level.
pub const SUN_RADIUS: f32 = 10_000.0;

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
}

/// A sun: DynLight has no directional lights, so this is a point light [`SUN_DISTANCE`] away
/// against `direction` (the way the light travels), with a radius that reaches the whole
/// level. Its rays are nearly parallel and its baked shadows sharp. Spawn the returned bundle.
pub fn sun_light(direction: Dir3) -> (DynamicLight, Transform) {
    (
        DynamicLight {
            radius: SUN_RADIUS,
            ..default()
        },
        Transform::from_translation(-direction.as_vec3() * SUN_DISTANCE)
            .looking_to(direction, perpendicular(direction)),
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
            sun_light(Dir3::NEG_Y).0,
        ] {
            assert!(light.is_valid(), "{light:?}");
        }
        assert_eq!(DynamicLight::spot().light_type, LightType::Spot);
        let disco = DynamicLight::discoball();
        assert_eq!((disco.inner_cutoff, disco.outer_cutoff), (12.5, 14.0));
    }

    #[test]
    fn the_sun_shines_along_its_direction_and_reaches_the_level() {
        let direction = Dir3::new(Vec3::new(1.0, -2.0, 0.5)).unwrap();
        let (light, transform) = sun_light(direction);
        assert!((transform.translation.length() - SUN_DISTANCE).abs() < 1e-2);
        assert!(transform.forward().dot(direction.as_vec3()) > 0.9999);
        assert!(light.radius > SUN_DISTANCE * 2.0);
        let (_, straight_down) = sun_light(Dir3::NEG_Y);
        assert!(straight_down.forward().dot(Vec3::NEG_Y) > 0.9999);
        assert!(straight_down.translation.is_finite());
    }
}
