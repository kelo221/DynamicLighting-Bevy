//! The room, portal and reflection probe keys of glTF node extras: which helper a node stands
//! for, the mistakes that leave one out, and whether a helper mesh has the shape the keys assume.

use bevy::math::Vec3;
use serde_json::{Map, Value};

pub(super) const ROOM: &str = "dynlight_room";
pub(super) const PORTAL: &str = "dynlight_portal";
pub(super) const CLOSED: &str = "dynlight_portal_closed";
pub(super) const PROBE: &str = "dynlight_reflection_probe";

/// A culling helper a node stands for: a [`Room`](crate::portals::Room) filling the node's
/// local ±1 cube, or a [`Portal`](crate::portals::Portal) filling the ±1 square in its local XY
/// plane. The node's transform places and sizes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Helper {
    Room,
    Portal { closed: bool },
}

/// What a node's room, portal and probe keys ask for.
pub(super) struct Intent {
    /// A room, portal or probe key is on, so the node is meant as a helper and its light and
    /// static keys are dropped, even when a mistake leaves the helper out.
    pub intended: bool,
    pub helper: Option<Helper>,
    /// The node's local ±1 cube is a reflection probe's box; a room may share it.
    pub probe: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flag {
    Absent,
    On,
    Off,
    Invalid,
}

/// Only `true`, `false`, `1` and `0` count.
fn flag(map: &Map<String, Value>, key: &str) -> Flag {
    match map.get(key) {
        None => Flag::Absent,
        Some(Value::Bool(true)) => Flag::On,
        Some(Value::Bool(false)) => Flag::Off,
        Some(Value::Number(number)) if number.as_f64() == Some(1.0) => Flag::On,
        Some(Value::Number(number)) if number.as_f64() == Some(0.0) => Flag::Off,
        Some(_) => Flag::Invalid,
    }
}

/// Reads the room and portal keys. A mistake that could cull a visible light (an unreadable
/// room key, a room that is also a portal) leaves the helper out; one that can only make
/// culling keep more (an unreadable portal or closed key) keeps the open portal.
pub(super) fn intent(map: &Map<String, Value>, warnings: &mut Vec<String>) -> Intent {
    let room = match flag(map, ROOM) {
        Flag::On => true,
        Flag::Invalid => {
            warnings.push(format!(
                "{ROOM} must be true, false, 1 or 0, so no room was made"
            ));
            false
        }
        Flag::Absent | Flag::Off => false,
    };
    let portal = match flag(map, PORTAL) {
        Flag::On => true,
        Flag::Invalid => {
            warnings.push(format!(
                "{PORTAL} must be true, false, 1 or 0; made an open portal anyway"
            ));
            true
        }
        Flag::Absent | Flag::Off => false,
    };
    let closed = match (portal, flag(map, CLOSED)) {
        (true, Flag::On) => true,
        (true, Flag::Invalid) => {
            warnings.push(format!(
                "{CLOSED} must be true, false, 1 or 0, so the portal stays open"
            ));
            false
        }
        (false, Flag::On | Flag::Invalid) => {
            warnings.push(format!("{CLOSED} needs {PORTAL} on the same node; ignored"));
            false
        }
        _ => false,
    };
    let helper = match (room, portal) {
        (true, true) => {
            warnings.push(format!(
                "{ROOM} and {PORTAL} are both on, so neither a room nor a portal was made"
            ));
            None
        }
        (true, false) => Some(Helper::Room),
        (false, true) => Some(Helper::Portal { closed }),
        (false, false) => None,
    };
    let probe = match flag(map, PROBE) {
        Flag::On if portal => {
            warnings.push(format!(
                "a portal is not a box, so {PROBE} was ignored; put the probe on its own node"
            ));
            false
        }
        Flag::On => true,
        Flag::Invalid => {
            warnings.push(format!(
                "{PROBE} must be true, false, 1 or 0, so no reflection probe was made"
            ));
            false
        }
        Flag::Absent | Flag::Off => false,
    };
    Intent {
        intended: room || portal || probe,
        helper,
        probe,
    }
}

/// Whether a helper mesh spanning `min`..`max` has the shape the helper is read as: the unit
/// cube for a room, a ±1 square in local XY (any depth) for a portal. A mesh whose scale was
/// applied into its vertices fails, since the node's scale is what sizes the helper.
pub(super) fn helper_mesh_fits(helper: Helper, min: Vec3, max: Vec3) -> bool {
    const SLACK: f32 = 1e-3;
    match helper {
        Helper::Room => min.abs_diff_eq(-Vec3::ONE, SLACK) && max.abs_diff_eq(Vec3::ONE, SLACK),
        Helper::Portal { .. } => {
            (min + max).abs_diff_eq(Vec3::ZERO, SLACK)
                && (max - min)
                    .truncate()
                    .abs_diff_eq(bevy::math::Vec2::splat(2.0), SLACK)
        }
    }
}
