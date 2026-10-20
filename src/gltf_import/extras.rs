//! Reads DynLight settings from glTF node extras (Blender custom properties).
//!
//! All keys are optional. A node is a light when it has at least one of the light keys (the
//! rows from `dynlight_type` to `dynlight_fog`) and isn't a room or portal helper.
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
//! | `dynlight_cast_shadows` | bool, on mesh nodes, default `true`: `false` makes it cast no shadows, baked or realtime ([`NoShadowCasting`](crate::shared::NoShadowCasting)) |
//! | `dynlight_room` | `true`/`false`/`1`/`0`: the node's local ±1 cube is a [`Room`](crate::portals::Room) |
//! | `dynlight_portal` | `true`/`false`/`1`/`0`: the ±1 square in the node's local XY plane is a [`Portal`](crate::portals::Portal) |
//! | `dynlight_portal_closed` | `true`/`false`/`1`/`0`, with `dynlight_portal`: the portal starts [closed](crate::portals::ClosedPortal) |
//!
//! A room or portal helper is never a light and never baked, and its own meshes are removed.
//! Conflicting or unreadable keys, and unknown `dynlight_` keys, are reported in
//! [`NodeExtras::warnings`].

use bevy::prelude::*;
use serde_json::{Map, Value};

pub use super::helpers::Helper;
use super::helpers::{CLOSED, PORTAL, ROOM, intent};
use crate::effects::{LightEffect, LightEffectKind};
use crate::fog::{VolumetricLight, VolumetricShape};
use crate::light::{DynamicLight, LightType, Shimmer};

const STATIC: &str = "dynlight_static";
const CAST_SHADOWS: &str = "dynlight_cast_shadows";

/// The keys that make a node a light.
pub const LIGHT_KEYS: [&str; 13] = [
    "dynlight_type",
    "dynlight_color",
    "dynlight_size",
    "dynlight_intensity",
    "dynlight_radius",
    "dynlight_falloff",
    "dynlight_inner_cutoff",
    "dynlight_outer_cutoff",
    "dynlight_angular_diameter",
    "dynlight_realtime",
    "dynlight_shimmer",
    "dynlight_effect",
    "dynlight_fog",
];

/// What a node's extras ask for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeExtras {
    pub light: Option<DynamicLight>,
    pub effect: Option<LightEffect>,
    pub fog: Option<VolumetricLight>,
    pub bake_static: bool,
    /// `dynlight_cast_shadows` is `false`.
    pub no_shadow_casting: bool,
    pub helper: Option<Helper>,
    /// Mistakes in the extras, each with what was made instead.
    pub warnings: Vec<String>,
}

/// Parses a node's extras JSON. Keys without the `dynlight_` prefix are ignored; malformed
/// light values keep their defaults.
pub fn parse(json: &str) -> NodeExtras {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(json) else {
        return NodeExtras::default();
    };
    let mut extras = NodeExtras::default();
    let known = |key: &str| {
        LIGHT_KEYS.contains(&key) || [STATIC, CAST_SHADOWS, ROOM, PORTAL, CLOSED].contains(&key)
    };
    for key in map.keys() {
        if key.starts_with("dynlight_") && !known(key) {
            extras.warnings.push(format!("unknown key {key} ignored"));
        }
    }
    let intent = intent(&map, &mut extras.warnings);
    let light_keys: Vec<&str> = LIGHT_KEYS
        .into_iter()
        .filter(|key| map.contains_key(*key))
        .collect();
    if intent.intended {
        extras.helper = intent.helper;
        if !light_keys.is_empty() {
            let keys = light_keys.join(", ");
            extras.warnings.push(format!(
                "a room or portal is never a light, so {keys} were ignored"
            ));
        }
        if boolean(&map, STATIC) == Some(true) {
            extras.warnings.push(format!(
                "a room or portal is never baked, so {STATIC} was ignored"
            ));
        }
        if map.contains_key(CAST_SHADOWS) {
            extras.warnings.push(format!(
                "a room or portal casts no shadows, so {CAST_SHADOWS} was ignored"
            ));
        }
        return extras;
    }
    extras.bake_static = boolean(&map, STATIC).unwrap_or(false);
    extras.no_shadow_casting = !boolean(&map, CAST_SHADOWS).unwrap_or(true);
    if light_keys.is_empty() {
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
