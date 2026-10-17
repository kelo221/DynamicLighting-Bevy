//! Baking the test scene: native `--bake` (or B) writes `assets/lighting_test.dynlight`;
//! an existing bake is applied at startup. Browser runs automatically bake a small in-memory
//! Preview (shadows only, no bounce). `--triangle-ids` forces the primitive-index fallback, and
//! `--auto-features` compiles only the shader features the scene uses.

use bevy::prelude::*;
use dynlight::bake::{BakeFinished, BakePreset, BakeStatus};
use dynlight::baked_lighting::ActiveBake;
use dynlight::shared::{DynamicLightingSettings, FeatureSelection, TriangleIdSource};

use crate::baking::PendingBake;
use crate::{WARM_UP_SECONDS, WarmUp, baking, support};

const BAKE_ASSET: &str = "lighting_test.dynlight";

pub fn plugin(app: &mut App) {
    let bake_now = cfg!(target_arch = "wasm32") || support::args().any(|arg| arg == "--bake");
    if bake_now {
        // `--frames` starts counting once the bake is applied.
        app.insert_resource(WarmUp {
            ready_at: f32::INFINITY,
        });
    }
    let triangle_ids = support::args().any(|arg| arg == "--triangle-ids");
    let automatic = support::args().any(|arg| arg == "--auto-features");
    if triangle_ids || automatic {
        let defaults = DynamicLightingSettings::default();
        app.insert_resource(DynamicLightingSettings {
            triangle_ids: if triangle_ids {
                TriangleIdSource::VertexAttribute
            } else {
                defaults.triangle_ids
            },
            feature_selection: if automatic {
                FeatureSelection::Automatic
            } else {
                defaults.feature_selection
            },
            ..defaults
        });
    }
    app.add_plugins(baking::plugin)
        .insert_resource(BakeOnStart(bake_now))
        .add_systems(Startup, load_existing_bake)
        .add_systems(Update, (request_bake, apply_finished_bake));
}

#[derive(Resource)]
struct BakeOnStart(bool);

fn load_existing_bake(server: Res<AssetServer>, mut active: ResMut<ActiveBake>) {
    baking::load_existing(BAKE_ASSET, &server, &mut active);
}

fn request_bake(
    keys: Res<ButtonInput<KeyCode>>,
    status: Res<BakeStatus>,
    mut on_start: ResMut<BakeOnStart>,
    mut pending: ResMut<PendingBake>,
) {
    let wanted = std::mem::take(&mut on_start.0) || keys.just_pressed(KeyCode::KeyB);
    if wanted && !status.is_running() {
        pending.queue(
            BAKE_ASSET,
            if cfg!(target_arch = "wasm32") || support::args().any(|arg| arg == "--preview") {
                BakePreset::Preview
            } else {
                BakePreset::Full
            },
        );
    }
}

fn apply_finished_bake(
    mut finished: MessageReader<BakeFinished>,
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
                active.0 = Some(report.handle.clone());
            }
            Err(error) => error!("bake failed: {error}"),
        }
        warm_up.ready_at = time.elapsed_secs() + WARM_UP_SECONDS;
    }
}
