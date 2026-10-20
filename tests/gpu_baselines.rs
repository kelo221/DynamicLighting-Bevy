//! Visual baselines: still frames of the examples (`--capture`, see
//! `examples/support/capture.rs`) compared with the images in `tests/baselines/` using NVIDIA's
//! FLIP, a perceptual difference metric. They need a GPU, so they are ignored by default:
//!
//! `cargo test --test gpu_baselines -- --ignored --test-threads=1`
//!
//! A failing case leaves its new frame and FLIP's error map (bright where the difference shows)
//! in `target/tmp/baselines/`. After an intended change to the look, rerun with
//! `DYNLIGHT_BLESS=1` to replace the baselines, and look at every changed image before committing.

mod common;

#[cfg(windows)]
use common::flip::check_with_env;
use common::flip::{check, check_against};

const OVERVIEW: &str = "0,6,14,0,0.6,0";
/// In the corridor of `rooms`, facing the door of the middle north room.
const ROOMS_DOOR: &str = "13,1.6,-1.2,13.8,1.1,6";
/// At the west end of the corridor of `rooms`, looking along it.
const ROOMS_CORRIDOR: &str = "1,1.7,-0.8,14,1.2,1.4";
/// 5 cm in front of the middle north door's leaf, facing it.
const ROOMS_AT_THE_DOOR: &str = "13.5,1.5,1.43,13.8,1.1,6";

#[test]
#[ignore = "needs a GPU"]
fn lighting_test_overview() {
    check(
        "lighting_test_overview",
        "lighting_test",
        &["--bake", "--camera", OVERVIEW],
    );
}

/// The triangle-ID fallback must look exactly like the primitive-index path.
#[test]
#[ignore = "needs a GPU"]
fn lighting_test_triangle_ids_match_the_overview() {
    check_against(
        "lighting_test_triangle_ids",
        "lighting_test_overview",
        "lighting_test",
        &["--bake", "--triangle-ids", "--camera", OVERVIEW],
    );
}

/// `FeatureSelection::Automatic` must compile every path the scene uses, so the frame is the
/// same as with every path compiled in.
#[test]
#[ignore = "needs a GPU"]
fn lighting_test_automatic_features_match_the_overview() {
    check_against(
        "lighting_test_automatic_features",
        "lighting_test_overview",
        "lighting_test",
        &["--bake", "--auto-features", "--camera", OVERVIEW],
    );
}

/// Direct3D 12 without `dxcompiler.dll` falls back to the old FXC compiler, which rejects some
/// shaders that DXC and Vulkan accept (an array inside a struct written at a run-time index).
#[cfg(windows)]
#[test]
#[ignore = "needs a GPU"]
fn lighting_test_on_fxc_matches_the_overview() {
    check_with_env(
        "lighting_test_fxc",
        "lighting_test_overview",
        "lighting_test",
        &["--bake", "--camera", OVERVIEW],
        &[("WGPU_BACKEND", "dx12"), ("WGPU_DX12_COMPILER", "fxc")],
    );
}

/// Without a bake every mesh takes the dynamic-geometry path.
#[test]
#[ignore = "needs a GPU"]
fn lighting_test_unbaked() {
    check(
        "lighting_test_unbaked",
        "lighting_test",
        &["--camera", OVERVIEW],
    );
}

/// Without a bake, `FeatureSelection::Automatic` leaves out the light BVH and the bake's other
/// paths, and the frame stays the same.
#[test]
#[ignore = "needs a GPU"]
fn lighting_test_unbaked_automatic_features_match() {
    check_against(
        "lighting_test_unbaked_automatic",
        "lighting_test_unbaked",
        "lighting_test",
        &["--auto-features", "--camera", OVERVIEW],
    );
}

#[test]
#[ignore = "needs a GPU"]
fn lighting_test_light_count_view() {
    check(
        "lighting_test_light_count",
        "lighting_test",
        &[
            "--bake",
            "--debug-view",
            "light-count",
            "--camera",
            OVERVIEW,
        ],
    );
}

/// The ceiling panel: an area light's soft baked shadows and its fog shaft.
#[test]
#[ignore = "needs a GPU"]
fn lighting_test_panel() {
    check(
        "lighting_test_panel",
        "lighting_test",
        &["--bake", "--camera", "-6.0,6.0,6.8,-6.0,0.0,10.4"],
    );
}

/// The sphere and box fog volumes, the cookie and the first light types.
#[test]
#[ignore = "needs a GPU"]
fn lighting_test_fog() {
    check(
        "lighting_test_fog",
        "lighting_test",
        &["--bake", "--camera", "-12,4.5,11,-12,1,1"],
    );
}

/// Metallic shading: metals and dielectrics at growing roughness.
#[test]
#[ignore = "needs a GPU"]
fn lighting_test_metallic() {
    check(
        "lighting_test_metallic",
        "lighting_test",
        &["--bake", "--camera", "-3,1.6,6,-3,0.35,2.6"],
    );
}

/// Realtime shadow cubemaps of a point and a spot light, frozen mid-motion.
#[test]
#[ignore = "needs a GPU"]
fn realtime_shadows() {
    check("realtime_shadows", "realtime_shadows", &[]);
}

/// 100 baked lights over pillars: light BVH, per-triangle light lists and baked shadows.
#[test]
#[ignore = "needs a GPU"]
fn stress_lights_baked() {
    check(
        "stress_lights_baked",
        "stress_lights",
        &["--lights", "100", "--bake"],
    );
}

/// A baked soft sun and a baked sky over a courtyard with a porch, with a lamp; the ball and the
/// crate aren't baked and take the pergola's and a tree's shadows from the sun map and the sky's
/// from the sky probes, and cast theirs through the sun's realtime cascades.
#[test]
#[ignore = "needs a GPU"]
fn outdoor() {
    check("outdoor", "outdoor", &["--bake"]);
}

/// The ball's realtime sun shadow on the baked ground, next to the pergola's baked shadows.
#[test]
#[ignore = "needs a GPU"]
fn outdoor_moving_shadow() {
    check(
        "outdoor_moving_shadow",
        "outdoor",
        &["--bake", "--camera", "-4.6,2.4,0.6,-1.7,0.25,-2.4"],
    );
}

/// The sun's shader paths through FXC, which lighting_test's scene doesn't compile.
#[cfg(windows)]
#[test]
#[ignore = "needs a GPU"]
fn outdoor_on_fxc_matches() {
    check_with_env(
        "outdoor_fxc",
        "outdoor",
        "outdoor",
        &["--bake"],
        &[("WGPU_BACKEND", "dx12"), ("WGPU_DX12_COMPILER", "fxc")],
    );
}

/// Without a bake the sky shines unshadowed through the dynamic-geometry path and the sun's
/// cascades shadow everything.
#[test]
#[ignore = "needs a GPU"]
fn outdoor_unbaked() {
    check("outdoor_unbaked", "outdoor", &[]);
}

/// A room seen through its door from the corridor: a shadowed lamp, a crate's shadow and a cone
/// of fog. Portal culling leaves out the lights, cubes and fog of the rooms out of sight.
#[test]
#[ignore = "needs a GPU"]
fn rooms() {
    check("rooms", "rooms", &["--camera", ROOMS_DOOR]);
}

/// What portal culling leaves out lights nothing in view.
#[test]
#[ignore = "needs a GPU"]
fn rooms_without_portals_match() {
    check_against(
        "rooms_no_portals",
        "rooms",
        "rooms",
        &["--no-portals", "--camera", ROOMS_DOOR],
    );
}

/// The corridor with every door shut: each leaf seals its doorway.
#[test]
#[ignore = "needs a GPU"]
fn rooms_closed() {
    check(
        "rooms_closed",
        "rooms",
        &["--closed-doors", "--camera", ROOMS_CORRIDOR],
    );
}

/// What a shut door leaves out lights nothing in view.
#[test]
#[ignore = "needs a GPU"]
fn rooms_closed_without_portals_match() {
    check_against(
        "rooms_closed_no_portals",
        "rooms_closed",
        "rooms",
        &["--closed-doors", "--no-portals", "--camera", ROOMS_CORRIDOR],
    );
}

/// A camera closer to a shut door than its near plane sees the room behind it, and that room's
/// fog spot, whose sphere stays inside the room.
#[test]
#[ignore = "needs a GPU"]
fn rooms_near_clip() {
    check(
        "rooms_near_clip",
        "rooms",
        &["--closed-doors", "--camera", ROOMS_AT_THE_DOOR],
    );
}

/// Portal views start in every room the near plane reaches, so nothing in sight is left out.
#[test]
#[ignore = "needs a GPU"]
fn rooms_near_clip_without_portals_match() {
    check_against(
        "rooms_near_clip_no_portals",
        "rooms_near_clip",
        "rooms",
        &[
            "--closed-doors",
            "--no-portals",
            "--camera",
            ROOMS_AT_THE_DOOR,
        ],
    );
}

/// `SingleThreadedSchedulesPlugin` changes how systems are run, not what they do: the same frames
/// with the bake, effects, fog and cookies, the shadow driver, the sun's cascades and the sky,
/// and portal culling.
#[test]
#[ignore = "needs a GPU"]
fn single_threaded_schedules_match() {
    let cases: [(&str, &str, &[&str]); 5] = [
        (
            "lighting_test_overview",
            "lighting_test",
            &["--bake", "--camera", OVERVIEW],
        ),
        ("realtime_shadows", "realtime_shadows", &[]),
        (
            "stress_lights_baked",
            "stress_lights",
            &["--lights", "100", "--bake"],
        ),
        ("outdoor", "outdoor", &["--bake"]),
        ("rooms", "rooms", &["--camera", ROOMS_DOOR]),
    ];
    for (baseline, example, args) in cases {
        let mut args = args.to_vec();
        args.push("--single-threaded");
        check_against(
            &format!("{baseline}_single_threaded"),
            baseline,
            example,
            &args,
        );
    }
}
