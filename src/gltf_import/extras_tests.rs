//! Node extras parsing: lights, the room and portal keys, and the mistakes they report.

use bevy::prelude::*;

use super::extras::{Helper, LIGHT_KEYS, NodeExtras, parse};
use super::helpers::helper_mesh_fits;
use crate::effects::LightEffectKind;
use crate::fog::VolumetricShape;
use crate::light::{DynamicLight, LightType};

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
    assert!(extras.warnings.is_empty(), "{:?}", extras.warnings);
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
fn meshes_cast_shadows_unless_told_not_to() {
    assert!(!parse(r#"{"dynlight_static": true}"#).no_shadow_casting);
    assert!(!parse(r#"{"dynlight_cast_shadows": true}"#).no_shadow_casting);
    for off in ["false", "0"] {
        let extras = parse(&format!(r#"{{"dynlight_cast_shadows": {off}}}"#));
        assert!(extras.no_shadow_casting, "{off}");
        assert!(extras.light.is_none() && extras.warnings.is_empty());
    }
    let room = parse(r#"{"dynlight_room": true, "dynlight_cast_shadows": false}"#);
    assert!(!room.no_shadow_casting);
    assert_eq!(room.warnings.len(), 1, "{:?}", room.warnings);
}

#[test]
fn unrelated_and_malformed_extras_are_ignored() {
    assert_eq!(parse(r#"{"foo": 1}"#), NodeExtras::default());
    assert_eq!(parse("not json"), NodeExtras::default());
    let extras = parse(r#"{"dynlight_type": "laser"}"#);
    assert_eq!(extras.light.unwrap().light_type, LightType::Point);
}

#[test]
fn every_light_key_alone_makes_a_light() {
    for key in LIGHT_KEYS {
        let extras = parse(&format!(r#"{{"{key}": 1}}"#));
        assert!(extras.light.is_some(), "{key}");
    }
}

#[test]
fn an_unknown_key_is_reported_and_makes_no_light() {
    let extras = parse(r#"{"dynlight_rooom": true, "dynlight_static": true}"#);
    assert!(extras.light.is_none() && extras.helper.is_none());
    assert!(extras.bake_static);
    assert_eq!(extras.warnings, ["unknown key dynlight_rooom ignored"]);
}

#[test]
fn rooms_and_portals_read_strict_flags() {
    for on in ["true", "1", "1.0"] {
        let room = parse(&format!(r#"{{"dynlight_room": {on}}}"#));
        assert_eq!(room.helper, Some(Helper::Room), "{on}");
        assert!(room.warnings.is_empty());
        let shut = parse(&format!(
            r#"{{"dynlight_portal": {on}, "dynlight_portal_closed": {on}}}"#
        ));
        assert_eq!(shut.helper, Some(Helper::Portal { closed: true }), "{on}");
    }
    for off in ["false", "0"] {
        let open = parse(&format!(
            r#"{{"dynlight_portal": true, "dynlight_portal_closed": {off}}}"#
        ));
        assert_eq!(open.helper, Some(Helper::Portal { closed: false }));
        assert_eq!(
            parse(&format!(r#"{{"dynlight_room": {off}}}"#)).helper,
            None
        );
    }
}

/// One row of the conflict table: the extras, then what is made and how many warnings.
fn row(json: &str) -> (Option<Helper>, bool, bool, usize) {
    let extras = parse(json);
    let warnings = extras.warnings.len();
    (
        extras.helper,
        extras.light.is_some(),
        extras.bake_static,
        warnings,
    )
}

#[test]
fn a_helper_drops_its_light_effect_fog_and_static_keys() {
    let room = parse(
        r#"{"dynlight_room": true, "dynlight_radius": 4, "dynlight_effect": "candle",
            "dynlight_fog": "sphere", "dynlight_static": true}"#,
    );
    assert_eq!(room.helper, Some(Helper::Room));
    assert!(room.light.is_none() && room.effect.is_none() && room.fog.is_none());
    assert!(!room.bake_static);
    assert_eq!(room.warnings.len(), 2, "{:?}", room.warnings);
    assert!(room.warnings[0].contains("dynlight_radius, dynlight_effect, dynlight_fog"));
    assert_eq!(
        row(r#"{"dynlight_portal": 1, "dynlight_type": "spot"}"#),
        (Some(Helper::Portal { closed: false }), false, false, 1)
    );
}

#[test]
fn a_room_that_is_also_a_portal_makes_neither() {
    assert_eq!(
        row(r#"{"dynlight_room": true, "dynlight_portal": true, "dynlight_radius": 2}"#),
        (None, false, false, 2)
    );
}

#[test]
fn closed_without_a_portal_makes_nothing() {
    assert_eq!(
        row(r#"{"dynlight_portal_closed": true}"#),
        (None, false, false, 1)
    );
    assert_eq!(
        row(r#"{"dynlight_room": true, "dynlight_portal_closed": 1}"#),
        (Some(Helper::Room), false, false, 1)
    );
    assert_eq!(
        row(r#"{"dynlight_portal": false, "dynlight_portal_closed": false}"#),
        (None, false, false, 0)
    );
}

#[test]
fn switched_off_helper_keys_leave_a_normal_light() {
    assert_eq!(
        row(r#"{"dynlight_room": false, "dynlight_radius": 3}"#),
        (None, true, false, 0)
    );
    assert_eq!(
        row(r#"{"dynlight_portal": 0, "dynlight_static": true}"#),
        (None, false, true, 0)
    );
}

#[test]
fn unreadable_values_fail_toward_culling_less() {
    for bad in [r#""yes""#, "2", "0.5", "[1]", "null"] {
        let room = format!(r#"{{"dynlight_room": {bad}, "dynlight_radius": 3}}"#);
        assert_eq!(row(&room), (None, true, false, 1), "room {bad}");
        let portal = format!(r#"{{"dynlight_portal": {bad}}}"#);
        assert_eq!(
            row(&portal),
            (Some(Helper::Portal { closed: false }), false, false, 1),
            "portal {bad}"
        );
        let closed = format!(r#"{{"dynlight_portal": true, "dynlight_portal_closed": {bad}}}"#);
        assert_eq!(
            row(&closed),
            (Some(Helper::Portal { closed: false }), false, false, 1),
            "closed {bad}"
        );
    }
}

#[test]
fn helper_meshes_must_keep_their_unit_shape() {
    let room = Helper::Room;
    assert!(helper_mesh_fits(room, -Vec3::ONE, Vec3::ONE));
    assert!(!helper_mesh_fits(
        room,
        Vec3::new(-3.0, -1.0, -1.0),
        Vec3::new(3.0, 1.0, 1.0)
    ));
    assert!(!helper_mesh_fits(room, Vec3::ZERO, Vec3::splat(2.0)));
    let portal = Helper::Portal { closed: false };
    assert!(helper_mesh_fits(
        portal,
        Vec3::new(-1.0, -1.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0)
    ));
    assert!(helper_mesh_fits(portal, -Vec3::ONE, Vec3::ONE));
    assert!(!helper_mesh_fits(
        portal,
        Vec3::new(-1.0, 0.0, -1.0),
        Vec3::new(1.0, 0.0, 1.0)
    ));
    assert!(!helper_mesh_fits(
        portal,
        Vec3::new(-0.6, -1.1, 0.0),
        Vec3::new(0.6, 1.1, 0.0)
    ));
}

#[test]
fn probes_fill_their_box_alone_or_with_a_room() {
    let probe = parse(r#"{"dynlight_reflection_probe": true, "dynlight_probe_diffuse": 0.4}"#);
    assert_eq!((probe.helper, probe.warnings.len()), (None, 0));
    let diffuse = probe.reflection_probe.map(|probe| probe.diffuse_intensity);
    assert_eq!(diffuse, Some(0.4));
    let room =
        parse(r#"{"dynlight_room": 1, "dynlight_reflection_probe": 1, "dynlight_radius": 2}"#);
    assert_eq!(room.helper, Some(Helper::Room));
    assert!(room.reflection_probe.is_some() && room.light.is_none());
    assert_eq!(room.warnings.len(), 1, "{:?}", room.warnings);
}

#[test]
fn probe_mistakes_make_no_probe() {
    let portal = parse(r#"{"dynlight_portal": true, "dynlight_reflection_probe": true}"#);
    assert_eq!(portal.helper, Some(Helper::Portal { closed: false }));
    assert!(portal.reflection_probe.is_none());
    assert_eq!(portal.warnings.len(), 1, "{:?}", portal.warnings);
    let unreadable = parse(r#"{"dynlight_reflection_probe": "yes"}"#);
    assert!(unreadable.reflection_probe.is_none() && unreadable.warnings.len() == 1);
    let alone = parse(r#"{"dynlight_probe_diffuse": 0.5, "dynlight_static": true}"#);
    assert!(alone.reflection_probe.is_none() && alone.bake_static);
    assert_eq!(alone.warnings.len(), 1, "{:?}", alone.warnings);
}
