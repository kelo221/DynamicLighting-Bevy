//! Builds the realtime cluster lists after the light buffer is packed and uploads them.

use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;

use super::assign::{ClusterLists, GridOff, build};
use super::buffer::REALTIME_CLUSTERS;
use crate::light::PackedLights;
use crate::shared::DynamicLightingSettings;

/// This frame's cluster lists, and why the grid is off when it is.
#[derive(Resource, Default, Debug)]
pub struct RealtimeClusters {
    pub lists: ClusterLists,
    pub off: Option<GridOff>,
}

pub(super) fn build_clusters(
    settings: Res<DynamicLightingSettings>,
    packed: Res<PackedLights>,
    mut clusters: ResMut<RealtimeClusters>,
) {
    let baked = packed.globals.baked_count as usize;
    let realtime = packed.lights.get(baked..).unwrap_or_default();
    let built = build(
        packed.viewer,
        realtime,
        settings.realtime_cluster_min_lights,
    );
    if built == Err(GridOff::TooManyEntries) && clusters.off != Some(GridOff::TooManyEntries) {
        warn!(
            "dynlight clusters: the {} realtime lights reach too many cells; every fragment \
             visits every realtime light",
            realtime.len()
        );
    }
    let clusters = &mut *clusters;
    match built {
        Ok(lists) => {
            clusters.lists = lists;
            clusters.off = None;
        }
        Err(reason) => {
            clusters.lists = ClusterLists::default();
            clusters.off = Some(reason);
        }
    }
}

pub(super) fn upload_clusters(
    clusters: Res<RealtimeClusters>,
    buffers: Option<ResMut<Assets<ShaderBuffer>>>,
) {
    let Some(mut buffers) = buffers else {
        return;
    };
    let Some(mut buffer) = buffers.get_mut(&REALTIME_CLUSTERS) else {
        return;
    };
    buffer.clear();
    buffer.extend_from_slice(&clusters.lists.to_bytes());
}
