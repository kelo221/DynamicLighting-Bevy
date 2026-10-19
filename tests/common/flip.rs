//! The visual baselines' comparison: a captured frame against `tests/baselines/<name>.png`
//! using NVIDIA's FLIP, a perceptual difference metric (see `tests/gpu_baselines.rs`).

use std::fs;
use std::path::{Path, PathBuf};

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, Image, ImageSampler, ImageType};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use nv_flip::{FlipImageRgb8, FlipPool};

use super::run_example_with_env;

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

pub fn check(name: &str, example: &str, args: &[&str]) {
    check_against(name, name, example, args);
}

pub fn check_against(name: &str, baseline: &str, example: &str, args: &[&str]) {
    check_with_env(name, baseline, example, args, &[]);
}

/// Captures `example` with `args` and the environment variables `env`, and compares the frame
/// with `tests/baselines/<baseline>.png`.
pub fn check_with_env(
    name: &str,
    baseline: &str,
    example: &str,
    args: &[&str],
    env: &[(&str, &str)],
) {
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("baselines");
    fs::create_dir_all(&out).expect("the output folder can be created");
    let frame = out.join(format!("{name}.png"));
    fs::remove_file(&frame).ok();
    let frame_arg = frame.to_string_lossy().into_owned();
    let mut capture_args = args.to_vec();
    capture_args.extend(["--capture", &frame_arg]);
    run_example_with_env(example, &capture_args, env);

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
