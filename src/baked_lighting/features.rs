//! Whether the GPU tells fragment shaders which triangle they belong to.
//!
//! Baked shadows are looked up per triangle through `@builtin(primitive_index)`, which needs
//! wgpu's `PRIMITIVE_INDEX` feature. Bevy requests every adapter feature by default
//! (`WgpuSettingsPriority::Functionality`); apps that pick features themselves must include it.
//! Without it, baked meshes get a triangle-ID vertex attribute instead (`triangle_ids`).

use core::sync::atomic::{AtomicU8, Ordering};

use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use bevy::render::settings::WgpuFeatures;

const UNKNOWN: u8 = 0;
const SUPPORTED: u8 = 1;
const MISSING: u8 = 2;

static PRIMITIVE_INDEX: AtomicU8 = AtomicU8::new(UNKNOWN);

/// Whether the render device supports `@builtin(primitive_index)`. False until the renderer
/// has started, and in headless apps.
pub fn primitive_index_supported() -> bool {
    PRIMITIVE_INDEX.load(Ordering::Relaxed) == SUPPORTED
}

/// Whether a render device has started without `@builtin(primitive_index)`.
pub(super) fn primitive_index_missing() -> bool {
    PRIMITIVE_INDEX.load(Ordering::Relaxed) == MISSING
}

/// Runs in `RenderStartup`, i.e. again whenever the render device is recreated.
pub(super) fn detect_features(device: Res<RenderDevice>) {
    let supported = device.features().contains(WgpuFeatures::PRIMITIVE_INDEX);
    if supported {
        debug!("primitive indices are available for baked shadows");
    } else {
        info!(
            "the GPU doesn't support primitive indices (wgpu `Features::PRIMITIVE_INDEX`); \
             baked meshes are de-indexed and carry triangle IDs as a vertex attribute instead"
        );
    }
    PRIMITIVE_INDEX.store(
        if supported { SUPPORTED } else { MISSING },
        Ordering::Relaxed,
    );
}
