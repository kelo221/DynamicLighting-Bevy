//! ECS orchestration: a [`BakeRequest`] snapshots the scene, bakes it on the
//! [`AsyncComputeTaskPool`] and writes the `.dynlight` file; [`BakeFinished`] reports the result.
//!
//! Without Bevy's `multi_threaded` feature the async compute pool runs on the main thread, so
//! the bake blocks that frame until it is done.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on};

use crate::shared::DynamicLightingSettings;

use super::format::{BakeStats, encode};
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

/// Bakes the current scene into `output` (e.g. `assets/level.dynlight`).
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct BakeRequest {
    pub output: PathBuf,
    pub preset: BakePreset,
}

/// Sent once per [`BakeRequest`] when its bake ends.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct BakeFinished {
    pub output: PathBuf,
    pub result: Result<BakeReport, BakeFailure>,
}

/// A successful bake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BakeReport {
    pub stats: BakeStats,
    /// Wall-clock time of the bake and the file write.
    pub milliseconds: u64,
    /// Size of the written file.
    pub bytes: u64,
}

/// Why a bake didn't produce a file.
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
    output: PathBuf,
    progress: Arc<BakeProgress>,
    task: Task<Result<BakeReport, BakeFailure>>,
}

impl BakeStatus {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Progress of the running bake.
    pub fn progress(&self) -> Option<&BakeProgress> {
        self.running.as_ref().map(|bake| &*bake.progress)
    }

    /// Where the running bake will be written.
    pub fn output(&self) -> Option<&Path> {
        self.running.as_ref().map(|bake| bake.output.as_path())
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
            error!(
                "dynlight bake: {} failed: {failure}",
                request.output.display()
            );
            BakeFinished {
                output: request.output.clone(),
                result: Err(failure),
            }
        };
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
            "dynlight bake: started {:?}, meshes {}, lights {}, output {}",
            request.preset,
            input.meshes.len(),
            input.lights.len(),
            request.output.display()
        );
        let progress = Arc::new(BakeProgress::new());
        let task = {
            let (progress, output) = (progress.clone(), request.output.clone());
            AsyncComputeTaskPool::get()
                .spawn(async move { bake_to_file(&input, &options, &progress, &output) })
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
    let result = block_on(bake.task);
    match &result {
        Ok(report) => info!(
            "dynlight bake: meshes {}, texels {}, rays {}, ms {}",
            report.stats.meshes, report.stats.texels, report.stats.rays, report.milliseconds
        ),
        Err(failure) => error!("dynlight bake: {} failed: {failure}", bake.output.display()),
    }
    finished.write(BakeFinished {
        output: bake.output,
        result,
    });
}

/// Bakes and writes the file through a temporary sibling, so asset hot reload never sees a
/// partial file.
fn bake_to_file(
    input: &BakeInput,
    options: &BakeOptions,
    progress: &BakeProgress,
    output: &Path,
) -> Result<BakeReport, BakeFailure> {
    let start = Instant::now();
    let scene = super::bake(input, options, progress).map_err(BakeFailure::Bake)?;
    let bytes = encode(&scene);
    let write_error =
        |error: std::io::Error| BakeFailure::Write(format!("{}: {error}", output.display()));
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(write_error)?;
    }
    let mut temporary = output.as_os_str().to_owned();
    temporary.push(".partial");
    std::fs::write(&temporary, &bytes).map_err(write_error)?;
    std::fs::rename(&temporary, output).map_err(write_error)?;
    Ok(BakeReport {
        stats: scene.stats,
        milliseconds: start.elapsed().as_millis() as u64,
        bytes: bytes.len() as u64,
    })
}
