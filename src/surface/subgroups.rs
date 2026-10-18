//! Whether lit pipelines get the `SUBGROUPS` shader def, which lets the pixels of a subgroup
//! that share a baked triangle or a cell of realtime lights load each light once
//! (`lighting.wesl`). Needs wgpu's `Features::SUBGROUP`, which Bevy requests whenever the
//! adapter has it.

use core::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::settings::WgpuFeatures;

use crate::shared::SubgroupUse;

static SUBGROUPS: AtomicBool = AtomicBool::new(false);

/// Whether lit pipelines use subgroup operations. False until the renderer has started, and in
/// headless apps.
pub(super) fn subgroups_enabled() -> bool {
    SUBGROUPS.load(Ordering::Relaxed)
}

fn use_subgroups(setting: SubgroupUse, supported: bool) -> bool {
    setting == SubgroupUse::Auto && supported
}

/// A `RenderStartup` system (it runs again whenever the render device is recreated) for the
/// setting the app was built with.
pub(super) fn detect_subgroups(setting: SubgroupUse) -> impl FnMut(Res<RenderDevice>) {
    move |device: Res<RenderDevice>| {
        let supported = device.features().contains(WgpuFeatures::SUBGROUP);
        let enabled = use_subgroups(setting, supported);
        if enabled {
            debug!("the lit shader shares light lookups across subgroups");
        } else if supported {
            debug!("subgroup operations are off by `DynamicLightingSettings::subgroups`");
        } else {
            debug!("the GPU has no subgroup operations (wgpu `Features::SUBGROUP`)");
        }
        SUBGROUPS.store(enabled, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subgroups_need_the_feature_and_the_setting() {
        assert!(use_subgroups(SubgroupUse::Auto, true));
        assert!(!use_subgroups(SubgroupUse::Auto, false));
        assert!(!use_subgroups(SubgroupUse::Off, true));
    }
}
