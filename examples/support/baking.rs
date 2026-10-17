//! Queue bakes long enough for the browser to paint the status before CPU work starts.

use std::path::PathBuf;

use bevy::prelude::*;
use dynlight::bake::{BakeFinished, BakePreset, BakeRequest, BakeStatus};
use dynlight::baked_lighting::ActiveBake;

use crate::support;

#[derive(Resource, Default)]
pub struct PendingBake {
    request: Option<BakeRequest>,
    frames: u32,
    seconds: f32,
}

impl PendingBake {
    pub fn is_pending(&self) -> bool {
        self.request.is_some()
    }

    pub fn queue(&mut self, asset: &str, preset: BakePreset) {
        if self.is_pending() {
            return;
        }
        let detail = match preset {
            BakePreset::Preview => "Preview, shadows only; bounce skipped",
            BakePreset::Full => "Full, including bounce",
        };
        support::status(&format!("Baking... ({detail})"));
        info!("baking {asset} ({detail})");
        self.request = Some(BakeRequest {
            output: output(asset),
            preset,
        });
        self.frames = 0;
        self.seconds = 0.0;
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<PendingBake>()
        .add_systems(Update, (dispatch, show_finished));
}

/// Where a bake is written. Browser bakes and `--capture` runs stay in memory, so a capture never
/// replaces the bake a normal run uses.
fn output(asset: &str) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        (!support::args().any(|arg| arg == "--capture"))
            .then(|| PathBuf::from("assets").join(asset))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = asset;
        None
    }
}

pub fn load_existing(asset: &str, server: &AssetServer, active: &mut ActiveBake) {
    #[cfg(not(target_arch = "wasm32"))]
    if output(asset).is_some_and(|path| path.exists()) {
        active.0 = Some(server.load(asset.to_owned()));
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (asset, server, active);
}

fn dispatch(
    mut pending: ResMut<PendingBake>,
    status: Res<BakeStatus>,
    time: Res<Time<Real>>,
    mut requests: MessageWriter<BakeRequest>,
) {
    if !pending.is_pending() || status.is_running() {
        return;
    }
    pending.frames += 1;
    pending.seconds += time.delta_secs();
    // At least two frame boundaries plus a short delay let the DOM and canvas paint first.
    if cfg!(target_arch = "wasm32") && (pending.frames < 3 || pending.seconds < 0.1) {
        return;
    }
    if let Some(request) = pending.request.take() {
        requests.write(request);
    }
}

fn show_finished(mut finished: MessageReader<BakeFinished>) {
    for bake in finished.read() {
        match &bake.result {
            Ok(report) => support::status(&format!(
                "Bake finished in {} ms: {} meshes, {} texels, {} rays",
                report.milliseconds, report.stats.meshes, report.stats.texels, report.stats.rays
            )),
            Err(error) => support::status(&format!("Bake failed: {error}")),
        }
    }
}
