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

use std::fs;
use std::path::{Path, PathBuf};

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, Image, ImageSampler, ImageType};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use nv_flip::{FlipImageRgb8, FlipPool};

/// FLIP's default viewing conditions: a 4K monitor 0.7 m wide, seen from 0.7 m.
const PIXELS_PER_DEGREE: f32 = 67.0;
/// A pixel whose FLIP error is above this shows a difference most viewers would notice.
const VISIBLE: f32 = 0.1;
/// The largest mean FLIP error a frame may have. Repeated runs are bit-identical, and Direct3D 12
/// against Vulkan stays below 0.00001 (RTX 4080 SUPER), while 3% darker albedo gives 0.016–0.038.
const MAX_MEAN: f32 = 0.002;
/// The largest share of pixels whose error is above [`VISIBLE`], for changes too local to move
/// the mean: a point light's shadow blur at σ 1 instead of 1.5 puts 1.1% of the pixels above it.
const MAX_VISIBLE_SHARE: f32 = 0.0001;

const OVERVIEW: &str = "0,6,14,0,0.6,0";
/// In the corridor of `rooms`, facing the door of the middle north room.
const ROOMS_DOOR: &str = "13,1.6,-1.2,13.8,1.1,6";

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

fn check(name: &str, example: &str, args: &[&str]) {
    check_against(name, name, example, args);
}

fn check_against(name: &str, baseline: &str, example: &str, args: &[&str]) {
    check_with_env(name, baseline, example, args, &[]);
}

/// Captures `example` with `args` and the environment variables `env`, and compares the frame
/// with `tests/baselines/<baseline>.png`.
fn check_with_env(name: &str, baseline: &str, example: &str, args: &[&str], env: &[(&str, &str)]) {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("baselines");
    fs::create_dir_all(&out).expect("the output folder can be created");
    let frame = out.join(format!("{name}.png"));
    fs::remove_file(&frame).ok();
    let frame_arg = frame.to_string_lossy().into_owned();
    let mut capture_args = args.to_vec();
    capture_args.extend(["--capture", &frame_arg]);
    common::run_example_with_env(example, &capture_args, env);

    let reference = baseline_path(baseline);
    if std::env::var_os("DYNLIGHT_BLESS").is_some() && name == baseline {
        fs::copy(&frame, &reference).expect("the baseline can be written");
        eprintln!("{name}: wrote {}", reference.display());
        return;
    }
    assert!(
        reference.exists(),
        "no baseline {}: run with DYNLIGHT_BLESS=1, then look at it",
        reference.display()
    );
    let (width, height, expected) = read_rgb(&reference);
    let (frame_width, frame_height, actual) = read_rgb(&frame);
    assert_eq!(
        (frame_width, frame_height),
        (width, height),
        "{name}: the frame and the baseline differ in size"
    );

    let error = nv_flip::flip(
        FlipImageRgb8::with_data(width, height, &expected),
        FlipImageRgb8::with_data(width, height, &actual),
        PIXELS_PER_DEGREE,
    );
    let mean = FlipPool::from_image(&error).mean();
    let values = error.to_vec();
    let visible = values.iter().filter(|value| **value > VISIBLE).count() as f32;
    let visible_share = visible / values.len() as f32;
    let max = values.iter().copied().fold(0.0, f32::max);
    let changed = expected
        .as_chunks::<3>()
        .0
        .iter()
        .zip(actual.as_chunks::<3>().0)
        .filter(|(a, b)| a != b)
        .count();
    let summary = format!(
        "{name}: {changed} pixels changed, mean FLIP {mean:.5}, {:.3}% of pixels above \
         {VISIBLE}, largest {max:.3}",
        visible_share * 100.0
    );
    eprintln!("{summary}");
    if mean > MAX_MEAN || visible_share > MAX_VISIBLE_SHARE {
        let map = out.join(format!("{name}.flip.png"));
        let colors = error.apply_color_lut(&nv_flip::magma_lut());
        write_rgb(&map, width, height, &colors.to_vec());
        panic!(
            "{summary} (allowed: mean {MAX_MEAN}, {}%). New frame: {}, error map: {}",
            MAX_VISIBLE_SHARE * 100.0,
            frame.display(),
            map.display()
        );
    }
}

fn baseline_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("baselines")
        .join(format!("{name}.png"))
}

/// Width, height and RGB bytes of a PNG.
fn read_rgb(path: &Path) -> (u32, u32, Vec<u8>) {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let image = Image::from_buffer(
        &bytes,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::all(),
    )
    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let rgb = image
        .data
        .as_deref()
        .expect("decoded pixels")
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
        .collect();
    (image.width(), image.height(), rgb)
}

fn write_rgb(path: &Path, width: u32, height: u32, rgb: &[u8]) {
    let rgba = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
        .collect();
    let image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::all(),
    );
    image
        .try_into_dynamic()
        .expect("an 8-bit image")
        .to_rgb8()
        .save(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
}
