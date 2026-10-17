//! GPU checks through the examples. They need a GPU and a display, which hosted CI runners
//! lack, so they are ignored by default: `cargo test --test gpu_examples -- --ignored`.

use std::path::{Path, PathBuf};
use std::process::Command;

use bevy::asset::RenderAssetUsages;
use bevy::image::{CompressedImageFormats, Image, ImageSampler, ImageType};

/// Runs `cargo run --example <name> -- <args>` with this test's features and asserts it exits
/// cleanly without logging render errors.
fn run_example(name: &str, args: &[&str]) -> String {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args(["run", "--example", name]);
    if cfg!(feature = "dev") {
        command.args(["--features", "dev"]);
    }
    let output = command.arg("--").args(args).output().expect("cargo runs");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{name} failed:\n{log}");
    assert!(!log.contains(" ERROR "), "{name} logged an error:\n{log}");
    log
}

fn scratch(file: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dynlight-gpu-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(file)
}

/// Mean and standard deviation of the screenshot's luminance, 0 to 1.
fn luminance_stats(path: &Path) -> (f32, f32) {
    let bytes = std::fs::read(path).expect("the screenshot was written");
    let image = Image::from_buffer(
        &bytes,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        true,
        ImageSampler::Default,
        RenderAssetUsages::all(),
    )
    .expect("the screenshot is a PNG");
    let data = image.data.expect("decoded pixels");
    let luminance: Vec<f32> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0)
        .collect();
    let mean = luminance.iter().sum::<f32>() / luminance.len() as f32;
    let variance = luminance
        .iter()
        .map(|l| (l - mean) * (l - mean))
        .sum::<f32>()
        / luminance.len() as f32;
    (mean, variance.sqrt())
}

#[test]
#[ignore = "needs a GPU"]
fn gpu_lighting_matches_the_cpu_mirror() {
    let log = run_example("gpu_check", &[]);
    assert!(log.contains("GPU lighting matches the CPU mirror"), "{log}");
}

/// 200 small lights seen in perspective, so the realtime lights come from the light grid's
/// per-cell lists, which must not miss any light that reaches a pixel. Runs at WebGPU's default
/// limits, where the lit material's fragment stage binds all 8 storage buffers allowed.
#[test]
#[ignore = "needs a GPU"]
fn gpu_clustered_lights_match_the_cpu_mirror() {
    let log = run_example("gpu_check", &["--many", "--webgpu-limits"]);
    assert!(log.contains("GPU lighting matches the CPU mirror"), "{log}");
    assert!(!log.contains(", 0 inside the light grid"), "{log}");
}

#[test]
#[ignore = "needs a GPU"]
fn gpu_specular_light_matches_the_cpu_mirror() {
    let log = run_example("gpu_check_specular", &[]);
    assert!(
        log.contains("GPU specular light matches the CPU mirror"),
        "{log}"
    );
}

#[test]
#[ignore = "needs a GPU"]
fn gpu_area_shadows_match_the_cpu_mirror_at_every_quality() {
    for quality in ["integrated", "low", "medium", "high"] {
        let log = run_example("gpu_check_area", &["--quality", quality]);
        assert!(
            log.contains("GPU area shadows match the CPU mirror"),
            "{log}"
        );
    }
}

/// The faces rendered per frame from the `realtime_shadows` log line.
fn faces_per_frame(log: &str) -> f32 {
    let line = log
        .lines()
        .find(|line| line.contains("cube faces rendered per frame"))
        .unwrap_or_else(|| panic!("no shadow report:\n{log}"));
    let after = line.split("ms per frame, ").nth(1).expect("report format");
    after
        .split(' ')
        .next()
        .and_then(|faces| faces.parse().ok())
        .expect("a face count")
}

#[test]
#[ignore = "needs a GPU"]
fn still_shadow_cubemaps_stay_cached() {
    let still = run_example("realtime_shadows", &["--frames", "120", "--still"]);
    assert_eq!(faces_per_frame(&still), 0.0, "{still}");
    // The orbiting lamp's six faces render every frame, the spot only the faces seeing crates.
    let moving = faces_per_frame(&run_example("realtime_shadows", &["--frames", "120"]));
    assert!((6.0..12.0).contains(&moving), "{moving} faces per frame");
}

#[test]
#[ignore = "needs a GPU"]
fn lighting_test_renders_a_lit_scene() {
    let lit = scratch("lit.png");
    let lit_path = lit.to_string_lossy().into_owned();
    run_example(
        "lighting_test",
        &["--frames", "30", "--screenshot", &lit_path],
    );
    let (mean, spread) = luminance_stats(&lit);
    // The clear color alone has a luminance of about 0.002 and no spread.
    assert!(mean > 0.05, "the scene is too dark: mean {mean}");
    assert!(spread > 0.03, "the scene is flat: spread {spread}");

    let count = scratch("light_count.png");
    let count_path = count.to_string_lossy().into_owned();
    run_example(
        "lighting_test",
        &[
            "--debug-view",
            "light-count",
            "--frames",
            "30",
            "--screenshot",
            &count_path,
        ],
    );
    let (count_mean, _) = luminance_stats(&count);
    assert!(
        (count_mean - mean).abs() > 0.01,
        "the debug view changed nothing"
    );
    std::fs::remove_dir_all(lit.parent().unwrap()).ok();
}
