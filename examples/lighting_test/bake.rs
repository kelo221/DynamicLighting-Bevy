//! Baking the test scene: `--bake` (or the B key) bakes it into
//! `assets/lighting_test.dynlight` and applies the result; an existing bake is applied at startup.
//! `--triangle-ids` draws baked meshes through the triangle-ID fallback that GPUs without
//! primitive indices use.

use std::path::PathBuf;

use bevy::prelude::*;
use dynlight::bake::{BakeFinished, BakePreset, BakeRequest, BakeStatus};
use dynlight::baked_lighting::ActiveBake;
use dynlight::shared::{DynamicLightingSettings, TriangleIdSource};

use crate::{WARM_UP_SECONDS, WarmUp};

const BAKE_ASSET: &str = "lighting_test.dynlight";

pub fn plugin(app: &mut App) {
    let bake_now = std::env::args().any(|arg| arg == "--bake");
    if bake_now {
        // `--frames` starts counting once the bake is applied.
        app.insert_resource(WarmUp {
            ready_at: f32::INFINITY,
        });
    }
    if std::env::args().any(|arg| arg == "--triangle-ids") {
        app.insert_resource(DynamicLightingSettings {
            triangle_ids: TriangleIdSource::VertexAttribute,
            ..default()
        });
    }
    app.insert_resource(BakeOnStart(bake_now))
        .add_systems(Startup, load_existing_bake)
        .add_systems(Update, (request_bake, apply_finished_bake));
}

#[derive(Resource)]
struct BakeOnStart(bool);

fn output() -> PathBuf {
    PathBuf::from("assets").join(BAKE_ASSET)
}

fn load_existing_bake(server: Res<AssetServer>, mut active: ResMut<ActiveBake>) {
    if output().exists() {
        active.0 = Some(server.load(BAKE_ASSET));
    }
}

fn request_bake(
    keys: Res<ButtonInput<KeyCode>>,
    status: Res<BakeStatus>,
    mut on_start: ResMut<BakeOnStart>,
    mut requests: MessageWriter<BakeRequest>,
) {
    let wanted = std::mem::take(&mut on_start.0) || keys.just_pressed(KeyCode::KeyB);
    if wanted && !status.is_running() {
        info!("baking into {}", output().display());
        requests.write(BakeRequest {
            output: output(),
            // `--preview` skips bounce lighting.
            preset: if std::env::args().any(|arg| arg == "--preview") {
                BakePreset::Preview
            } else {
                BakePreset::Full
            },
        });
    }
}

fn apply_finished_bake(
    mut finished: MessageReader<BakeFinished>,
    server: Res<AssetServer>,
    time: Res<Time<Real>>,
    mut active: ResMut<ActiveBake>,
    mut warm_up: ResMut<WarmUp>,
) {
    for bake in finished.read() {
        match &bake.result {
            Ok(report) => {
                info!(
                    "baked {} meshes, {} texels, {} rays in {} ms ({} bytes)",
                    report.stats.meshes,
                    report.stats.texels,
                    report.stats.rays,
                    report.milliseconds,
                    report.bytes
                );
                if active.0.is_some() {
                    server.reload(BAKE_ASSET);
                } else {
                    active.0 = Some(server.load(BAKE_ASSET));
                }
            }
            Err(error) => error!("bake failed: {error}"),
        }
        warm_up.ready_at = time.elapsed_secs() + WARM_UP_SECONDS;
    }
}
