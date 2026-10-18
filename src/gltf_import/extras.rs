//! Reads DynLight settings from glTF node extras (Blender custom properties).
//!
//! Keys, all optional except that at least one `dynlight_` light key makes a light:
//!
//! | Key | Value |
//! |---|---|
//! | `dynlight_type` | `point`, `spot`, `discoball`, `wave`, `interference`, `rotor`, `shock`, `disco`, `rectangle`, `tube`, `sun` |
//! | `dynlight_color` | `[r, g, b]` sRGB 0–1 |
//! | `dynlight_size` | `[width, height]` of a rectangle, `[length]` of a tube, in meters |
//! | `dynlight_intensity`, `dynlight_radius`, `dynlight_falloff` | numbers |
//! | `dynlight_inner_cutoff`, `dynlight_outer_cutoff` | degrees |
//! | `dynlight_angular_diameter` | degrees, a sun's disk (0 for hard shadows) |
//! | `dynlight_realtime` | bool |
//! | `dynlight_shimmer` | `none`, `water`, `random` |
//! | `dynlight_effect` | an effect name such as `candle` or `fluorescent_starter` |
//! | `dynlight_fog` | `sphere`, `box`, `cone_z`, `cone_y`, `frustum` |
//! | `dynlight_static` | bool, on mesh nodes: bake shadows onto and from this mesh |

use bevy::prelude::*;
use serde_json::{Map, Value};

use crate::effects::{LightEffect, LightEffectKind};
use crate::fog::{VolumetricLight, VolumetricShape};
use crate::light::{DynamicLight, LightType, Shimmer};

/// What a node's extras ask for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeExtras {
    pub light: Option<DynamicLight>,
    pub effect: Option<LightEffect>,
    pub fog: Option<VolumetricLight>,
    pub bake_static: bool,
}

/// Parses a node's extras JSON. Unknown keys and malformed values are ignored.
pub fn parse(json: &str) -> NodeExtras {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(json) else {
        return NodeExtras::default();
    };
    let mut extras = NodeExtras {
        bake_static: boolean(&map, "dynlight_static").unwrap_or(false),
        ..default()
    };
    let is_light = map
        .keys()
        .any(|key| key.starts_with("dynlight_") && key != "dynlight_static");
    if !is_light {
        return extras;
    }
    let mut light = DynamicLight::default();
    // A sky needs an image, which extras can't carry.
    let types = &LightType::ALL[..LightType::ALL.len() - 1];
    if let Some(kind) = name_of(&map, "dynlight_type", types) {
        light.light_type = kind;
    }
    if let [r, g, b, ..] = numbers(&map, "dynlight_color")[..] {
        light.color = Color::srgb(r, g, b);
    }
    match numbers(&map, "dynlight_size")[..] {
        [length] => light.size = Vec2::new(length, 0.0),
        [width, height, ..] => light.size = Vec2::new(width, height),
        [] => {}
    }
    let fields: [(&str, &mut f32); 6] = [
        ("dynlight_intensity", &mut light.intensity),
        ("dynlight_radius", &mut light.radius),
        ("dynlight_falloff", &mut light.falloff),
        ("dynlight_inner_cutoff", &mut light.inner_cutoff),
        ("dynlight_outer_cutoff", &mut light.outer_cutoff),
        ("dynlight_angular_diameter", &mut light.angular_diameter),
    ];
    for (key, field) in fields {
        if let Some(value) = number(&map, key) {
            *field = value;
        }
    }
    light.realtime = boolean(&map, "dynlight_realtime").unwrap_or(false);
    if let Some(shimmer) = name_of(
        &map,
        "dynlight_shimmer",
        &[Shimmer::None, Shimmer::Water, Shimmer::Random],
    ) {
        light.shimmer = shimmer;
    }
    extras.effect = name_of(&map, "dynlight_effect", &LightEffectKind::ALL).map(LightEffect::new);
    extras.fog = name_of(
        &map,
        "dynlight_fog",
        &[
            VolumetricShape::Sphere,
            VolumetricShape::Box,
            VolumetricShape::ConeZ,
            VolumetricShape::ConeY,
            VolumetricShape::Frustum,
        ],
    )
    .map(|shape| VolumetricLight { shape, ..default() });
    extras.light = Some(light);
    extras
}

fn number(map: &Map<String, Value>, key: &str) -> Option<f32> {
    map.get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
}

fn numbers(map: &Map<String, Value>, key: &str) -> Vec<f32> {
    match map.get(key) {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_f64)
            .map(|value| value as f32)
            .collect(),
        _ => Vec::new(),
    }
}

fn boolean(map: &Map<String, Value>, key: &str) -> Option<bool> {
    match map.get(key)? {
        Value::Bool(value) => Some(*value),
        // Blender stores booleans from older UIs as 0/1.
        Value::Number(value) => value.as_f64().map(|value| value != 0.0),
        _ => None,
    }
}

/// Matches a string value against the `Debug` names of `options`, ignoring case and `_`.
fn name_of<T: Copy + core::fmt::Debug>(
    map: &Map<String, Value>,
    key: &str,
    options: &[T],
) -> Option<T> {
    let wanted = normalize(map.get(key)?.as_str()?);
    options
        .iter()
        .copied()
        .find(|option| normalize(&format!("{option:?}")) == wanted)
}

fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '_' && *c != ' ' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_light_with_effect_and_fog() {
        let extras = parse(
            r#"{"dynlight_type": "spot", "dynlight_radius": 6, "dynlight_color": [1, 0.5, 0],
                "dynlight_effect": "fluorescent_starter", "dynlight_fog": "cone_z",
                "dynlight_realtime": true, "other": 3}"#,
        );
        let light = extras.light.unwrap();
        assert_eq!(light.light_type, LightType::Spot);
        assert_eq!(light.radius, 6.0);
        assert!(light.realtime);
        assert_eq!(light.color, Color::srgb(1.0, 0.5, 0.0));
        assert_eq!(
            extras.effect.unwrap().kind,
            LightEffectKind::FluorescentStarter
        );
        assert_eq!(extras.fog.unwrap().shape, VolumetricShape::ConeZ);
    }

    #[test]
    fn area_lights_take_a_size() {
        let panel = parse(r#"{"dynlight_type": "rectangle", "dynlight_size": [1.2, 0.6]}"#);
        let panel = panel.light.unwrap();
        assert_eq!(panel.light_type, LightType::Rectangle);
        assert_eq!(panel.size, Vec2::new(1.2, 0.6));
        let tube = parse(r#"{"dynlight_type": "tube", "dynlight_size": [2]}"#);
        assert_eq!(tube.light.unwrap().size, Vec2::new(2.0, 0.0));
        let shaft = parse(r#"{"dynlight_type": "rectangle", "dynlight_fog": "frustum"}"#);
        assert_eq!(shaft.fog.unwrap().shape, VolumetricShape::Frustum);
    }

    #[test]
    fn suns_take_an_angular_diameter() {
        let sun = parse(r#"{"dynlight_type": "sun", "dynlight_angular_diameter": 2.5}"#);
        let sun = sun.light.unwrap();
        assert_eq!(
            (sun.light_type, sun.angular_diameter),
            (LightType::Sun, 2.5)
        );
        let default = parse(r#"{"dynlight_type": "Sun"}"#).light.unwrap();
        assert_eq!(default.angular_diameter, DynamicLight::SUN_ANGULAR_DIAMETER);
    }

    #[test]
    fn static_meshes_are_not_lights() {
        let extras = parse(r#"{"dynlight_static": 1}"#);
        assert!(extras.bake_static);
        assert!(extras.light.is_none());
    }

    #[test]
    fn unrelated_and_malformed_extras_are_ignored() {
        assert_eq!(parse(r#"{"foo": 1}"#), NodeExtras::default());
        assert_eq!(parse("not json"), NodeExtras::default());
        let extras = parse(r#"{"dynlight_type": "laser"}"#);
        assert_eq!(extras.light.unwrap().light_type, LightType::Point);
    }
}
