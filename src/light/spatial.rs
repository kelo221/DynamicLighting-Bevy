//! The per-type general-purpose shader parameters `gp1..gp3`.

use core::f32::consts::{PI, TAU};

use super::{component::DynamicLight, kinds::LightType};

/// The three general-purpose floats the shaders read for `light`'s type at time `seconds`:
///
/// - spot, discoball: cosines of the inner and outer cutoff
/// - wave, shock: wave time and frequency
/// - interference: wave phase and frequency in radians
/// - rotor: wave phase, blade count and center size
/// - disco: horizontal phase, spot count and vertical phase
/// - rectangle, tube: half extents of the shape in its local X and Y
///
/// Spot lights with a cookie overwrite `gp3` with the cookie projection scale.
pub fn type_parameters(light: &DynamicLight, seconds: f32) -> [f32; 3] {
    let wave_time = light.wave_offset + seconds * light.wave_speed;
    match light.light_type {
        LightType::Point => [0.0; 3],
        LightType::Rectangle | LightType::Tube => {
            let half = light.half_extents();
            [half.x, half.y, 0.0]
        }
        LightType::Spot | LightType::Discoball => [
            light.inner_cutoff.to_radians().cos(),
            light.outer_cutoff.to_radians().cos(),
            0.0,
        ],
        LightType::Wave => [wave_time, light.wave_frequency * TAU, 0.0],
        LightType::Interference => [wave_time * TAU, light.wave_frequency * PI, 0.0],
        LightType::Rotor => [
            wave_time * TAU,
            light.wave_frequency.round(),
            light.rotor_center,
        ],
        LightType::Shock => [wave_time, light.wave_frequency, 0.0],
        LightType::Disco => [
            wave_time * TAU,
            light.wave_frequency.round(),
            (light.wave_offset + seconds * light.disco_vertical_speed) * TAU,
        ],
    }
}

/// Scale that projects a spot light's cookie texture across its outer cone.
pub fn cookie_projection_scale(outer_cutoff_degrees: f32) -> f32 {
    0.5 * (90.0 - outer_cutoff_degrees.clamp(0.0, 90.0))
        .to_radians()
        .tan()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(light_type: LightType) -> DynamicLight {
        DynamicLight {
            light_type,
            wave_speed: 2.0,
            wave_frequency: 2.6,
            wave_offset: 0.25,
            ..Default::default()
        }
    }

    #[test]
    fn point_has_no_parameters() {
        assert_eq!(type_parameters(&light(LightType::Point), 3.0), [0.0; 3]);
    }

    #[test]
    fn cone_types_store_cutoff_cosines() {
        for light_type in [LightType::Spot, LightType::Discoball] {
            let [inner, outer, _] = type_parameters(&light(light_type), 1.0);
            assert!((inner - 26.0_f32.to_radians().cos()).abs() < 1e-6);
            assert!((outer - 30.0_f32.to_radians().cos()).abs() < 1e-6);
            assert!(inner > outer, "inner cone is narrower");
        }
    }

    #[test]
    fn wave_types_advance_with_time() {
        let wave_time = 0.25 + 1.5 * 2.0;
        assert_eq!(
            type_parameters(&light(LightType::Wave), 1.5),
            [wave_time, 2.6 * TAU, 0.0]
        );
        assert_eq!(
            type_parameters(&light(LightType::Shock), 1.5),
            [wave_time, 2.6, 0.0]
        );
        assert_eq!(
            type_parameters(&light(LightType::Interference), 1.5),
            [wave_time * TAU, 2.6 * PI, 0.0]
        );
    }

    #[test]
    fn rotor_and_disco_round_their_blade_count() {
        let [_, blades, center] = type_parameters(&light(LightType::Rotor), 0.0);
        assert_eq!((blades, center), (3.0, 0.1));
        let [phase, spots, vertical] = type_parameters(&light(LightType::Disco), 2.0);
        assert_eq!(spots, 3.0);
        assert!((phase - (0.25 + 4.0) * TAU).abs() < 1e-4);
        assert!((vertical - (0.25 + 2.0) * TAU).abs() < 1e-4);
    }

    #[test]
    fn invalid_cones_are_rejected() {
        for light_type in [LightType::Spot, LightType::Discoball] {
            let mut cone = light(light_type);
            assert!(cone.is_valid());
            cone.outer_cutoff = cone.inner_cutoff - 1.0;
            assert!(!cone.is_valid());
            cone.inner_cutoff = 0.0;
            cone.outer_cutoff = 0.0;
            assert!(!cone.is_valid());
        }
        let point = DynamicLight {
            outer_cutoff: 0.0,
            ..Default::default()
        };
        assert!(point.is_valid());
    }

    #[test]
    fn cookie_scale_grows_as_the_cone_narrows() {
        assert!((cookie_projection_scale(45.0) - 0.5).abs() < 1e-6);
        assert!(cookie_projection_scale(10.0) > cookie_projection_scale(45.0));
    }
}
