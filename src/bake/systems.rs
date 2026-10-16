//! ECS orchestration: a [`BakeRequest`] snapshots the scene and bakes it on the
//! [`AsyncComputeTaskPool`]; [`BakeFinished`] reports the asset and optional file output.
//!
//! Without Bevy's `multi_threaded` feature the async compute pool runs on the main thread, so
//! the bake blocks that frame until it is done.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on};

use crate::shared::DynamicLightingSettings;

use super::asset::{DynLightBake, HeadlessBakes};
use super::format::{BakeStats, BakedScene, encode};
use super::input::{BakeError, BakeInput, BakeOptions};
use super::progress::BakeProgress;
use super::settings::BakeSettings;
use super::snapshot::{SceneSnapshot, SnapshotError};

/// Henry's two bake menus. The resolution choice is [`BakeSettings::max_lightmap_size`].
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum BakePreset {
    /// Shadows only: bounce lighting is skipped for a quick look.
    Preview,
    /// Everything, including bounce lighting.
    #[default]
    Full,
}

/// Bakes the current scene into an asset, optionally writing a `.dynlight` file.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct BakeRequest {
    /// `None` keeps the bake in memory without filesystem access, including in browsers.
    /// `Some` writes a file (e.g. `assets/level.dynlight`); unsupported on wasm.
    pub output: Option<PathBuf>,
    pub preset: BakePreset,
}

/// Sent once per [`BakeRequest`] when its bake ends.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct BakeFinished {
    pub output: Option<PathBuf>,
    pub result: Result<BakeReport, BakeFailure>,
}

/// A successful bake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeReport {
    /// The in-memory bake, ready to use with [`ActiveBake`](crate::baked_lighting::ActiveBake).
    pub handle: Handle<DynLightBake>,
    pub stats: BakeStats,
    /// Wall-clock time of baking, encoding and any file write.
    pub milliseconds: u64,
    /// Size of the encoded `.dynlight` data, even when no file was requested.
    pub bytes: u64,
}

/// Why a bake didn't produce an asset.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeFailure {
    /// Another bake is still running; only one runs at a time.
    Busy,
    Snapshot(SnapshotError),
    Bake(BakeError),
    /// The file couldn't be written.
    Write(String),
}

impl fmt::Display for BakeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy => write!(f, "another bake is still running"),
            Self::Snapshot(error) => error.fmt(f),
            Self::Bake(error) => error.fmt(f),
            Self::Write(error) => write!(f, "couldn't write the bake: {error}"),
        }
    }
}

impl std::error::Error for BakeFailure {}

/// The running bake, if any: progress and cancellation.
#[derive(Resource, Default)]
pub struct BakeStatus {
    running: Option<RunningBake>,
}

struct RunningBake {
    output: Option<PathBuf>,
    progress: Arc<BakeProgress>,
    task: Task<Result<BakeOutput, BakeFailure>>,
}

struct BakeOutput {
    scene: BakedScene,
    milliseconds: u64,
    bytes: u64,
}

impl BakeStatus {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Progress of the running bake.
    pub fn progress(&self) -> Option<&BakeProgress> {
        self.running.as_ref().map(|bake| &*bake.progress)
    }

    /// Where the running bake will be written; `None` for in-memory bakes or when idle.
    pub fn output(&self) -> Option<&Path> {
        self.running
            .as_ref()
            .and_then(|bake| bake.output.as_deref())
    }

    /// Asks the running bake to stop; it finishes with [`BakeError::Cancelled`] and writes
    /// nothing. Returns whether a bake was running.
    pub fn cancel(&self) -> bool {
        self.progress().inspect(|p| p.cancel()).is_some()
    }
}

/// The bake options for `preset`.
pub fn bake_options(
    preset: BakePreset,
    settings: &BakeSettings,
    lighting: &DynamicLightingSettings,
) -> BakeOptions {
    let mut options =
        BakeOptions::from_settings(settings, lighting.dynamic_geometry, lighting.seed);
    options.skip_bounce = preset == BakePreset::Preview;
    options
}

pub(super) fn start_bakes(
    mut requests: MessageReader<BakeRequest>,
    mut status: ResMut<BakeStatus>,
    snapshot: SceneSnapshot,
    settings: Res<BakeSettings>,
    lighting: Option<Res<DynamicLightingSettings>>,
    mut finished: MessageWriter<BakeFinished>,
) {
    for request in requests.read() {
        let fail = |failure: BakeFailure| {
            error!("dynlight bake: {:?} failed: {failure}", request.output);
            BakeFinished {
                output: request.output.clone(),
                result: Err(failure),
            }
        };
        if let Some(failure) = unsupported_output(request.output.as_deref(), HAS_FILESYSTEM) {
            finished.write(fail(failure));
            continue;
        }
        if status.is_running() {
            finished.write(fail(BakeFailure::Busy));
            continue;
        }
        let input = match snapshot.take() {
            Ok(input) => input,
            Err(error) => {
                finished.write(fail(BakeFailure::Snapshot(error)));
                continue;
            }
        };
        let scene_settings = lighting.as_deref().cloned().unwrap_or_default();
        let options = bake_options(request.preset, &settings, &scene_settings);
        info!(
            "dynlight bake: started {:?}, meshes {}, lights {}, output {:?}",
            request.preset,
            input.meshes.len(),
            input.lights.len(),
            request.output
        );
        let progress = Arc::new(BakeProgress::new());
        let task = {
            let (progress, output) = (progress.clone(), request.output.clone());
            AsyncComputeTaskPool::get()
                .spawn(async move { bake_output(&input, &options, &progress, output.as_deref()) })
        };
        status.running = Some(RunningBake {
            output: request.output.clone(),
            progress,
            task,
        });
    }
}

pub(super) fn finish_bakes(
    mut status: ResMut<BakeStatus>,
    mut assets: ResMut<Assets<DynLightBake>>,
    headless: Option<ResMut<HeadlessBakes>>,
    mut finished: MessageWriter<BakeFinished>,
) {
    if !status
        .running
        .as_ref()
        .is_some_and(|bake| bake.task.is_finished())
    {
        return;
    }
    let Some(bake) = status.running.take() else {
        return;
    };
    // The task is finished, so this returns immediately.
    let result = block_on(bake.task).map(|output| BakeReport {
        stats: output.scene.stats,
        handle: assets.add(DynLightBake {
            scene: output.scene,
        }),
        milliseconds: output.milliseconds,
        bytes: output.bytes,
    });
    match &result {
        Ok(report) => {
            info!(
                "dynlight bake: meshes {}, texels {}, rays {}, ms {}",
                report.stats.meshes, report.stats.texels, report.stats.rays, report.milliseconds
            );
            if let Some(mut headless) = headless {
                headless.hold(&report.handle);
            }
        }
        Err(failure) => error!("dynlight bake: {:?} failed: {failure}", bake.output),
    }
    finished.write(BakeFinished {
        output: bake.output,
        result,
    });
}

fn bake_output(
    input: &BakeInput,
    options: &BakeOptions,
    progress: &BakeProgress,
    output: Option<&Path>,
) -> Result<BakeOutput, BakeFailure> {
    let start = Instant::now();
    let scene = super::bake(input, options, progress).map_err(BakeFailure::Bake)?;
    let bytes = encode(&scene);
    if let Some(output) = output {
        write_output(output, &bytes)?;
    }
    Ok(BakeOutput {
        scene,
        milliseconds: start.elapsed().as_millis() as u64,
        bytes: bytes.len() as u64,
    })
}

/// Writes through a temporary sibling so asset hot reload never sees a partial file.
#[cfg(not(target_arch = "wasm32"))]
fn write_output(output: &Path, bytes: &[u8]) -> Result<(), BakeFailure> {
    let write_error =
        |error: std::io::Error| BakeFailure::Write(format!("{}: {error}", output.display()));
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(write_error)?;
    }
    let mut temporary = output.as_os_str().to_owned();
    temporary.push(".partial");
    std::fs::write(&temporary, bytes).map_err(write_error)?;
    std::fs::rename(&temporary, output).map_err(write_error)?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn write_output(output: &Path, _bytes: &[u8]) -> Result<(), BakeFailure> {
    Err(no_filesystem(output))
}

/// Browsers have no filesystem to write a bake to.
const HAS_FILESYSTEM: bool = !cfg!(target_arch = "wasm32");

fn no_filesystem(output: &Path) -> BakeFailure {
    BakeFailure::Write(format!(
        "{}: filesystem output is unsupported on wasm; use BakeRequest.output = None",
        output.display()
    ))
}

/// A file output the platform can't write, rejected before the bake runs.
fn unsupported_output(output: Option<&Path>, filesystem: bool) -> Option<BakeFailure> {
    output.filter(|_| !filesystem).map(no_filesystem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_output_without_a_filesystem_is_rejected_up_front() {
        let path = Path::new("assets/level.dynlight");
        assert!(matches!(
            unsupported_output(Some(path), false),
            Some(BakeFailure::Write(message)) if message.contains("level.dynlight")
        ));
        assert_eq!(unsupported_output(None, false), None);
        assert_eq!(unsupported_output(Some(path), true), None);
    }
}
