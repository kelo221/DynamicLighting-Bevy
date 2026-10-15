//! The CPU baker: light BVH, occluder BVH, per-texel shadow rays, seams, bounce and distance
//! cubes, written to a `.dynlight` asset.
//!
//! [`bake`] is the pure entry point; [`BakeRequest`] runs it from the ECS on the async compute
//! pool and returns an in-memory asset, optionally also writing a file. Baked meshes must keep
//! their main-world data
//! (`RenderAssetUsages::MAIN_WORLD`) so the bake can read their vertices.

mod asset;
mod baker;
mod bits;
mod bounce;
mod bounce_map;
mod bounce_pack;
mod distance_cubes;
pub mod format;
mod geometry;
mod identity;
mod input;
mod light_bvh;
mod mesh_bake;
mod mesh_builder;
pub mod mirror;
mod occluder;
mod progress;
mod random;
mod seams;
mod settings;
mod shadows;
mod snapshot;
mod systems;
#[cfg(test)]
mod tests;
mod texels;
mod triangle_words;

use bevy::prelude::*;
use bevy::transform::TransformSystems;

pub use asset::{DynLightBake, DynLightBakeLoader, DynLightBakeLoaderError};
pub use baker::bake;
pub use distance_cubes::{DISTANCE_CUBE_BIAS, DISTANCE_CUBE_RESOLUTION};
pub use format::{
    BakeStats, BakedLight, BakedMesh, BakedScene, BakedSettings, FORMAT_VERSION, FormatError,
    decode, encode,
};
pub use input::{AlphaTexture, BakeBounce, BakeError, BakeInput, BakeLight, BakeMesh, BakeOptions};
pub use light_bvh::{LIGHT_BVH_NODE_SIZE, LightBvhNode};
pub use progress::{BakeProgress, BakeStage};
pub use settings::{BakeSettings, BakeStatic};
pub use snapshot::{BakeAlphaMask, SnapshotError};
pub use systems::{
    BakeFailure, BakeFinished, BakePreset, BakeReport, BakeRequest, BakeStatus, bake_options,
};
pub use triangle_words::TRIANGLE_HEADER_WORDS;

/// Ordering of the bake slice's `PostUpdate` work.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BakeSystems {
    /// [`ResolvedBakeId`](crate::shared::ResolvedBakeId)s are brought up to date; runtime
    /// matching of bake data runs after this.
    ResolveIds,
}

/// Bake settings, [`BakeRequest`] handling and the `.dynlight` asset loader (when the app has an
/// `AssetPlugin`).
pub struct BakePlugin;

impl Plugin for BakePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BakeSettings>()
            .init_resource::<BakeStatus>()
            // Apps that update before `finish` still need it; with an asset server,
            // `asset::register` replaces it with a registered one.
            .init_resource::<Assets<DynLightBake>>()
            .add_message::<BakeRequest>()
            .add_message::<BakeFinished>()
            .add_systems(
                PostUpdate,
                (
                    identity::resolve_bake_ids.in_set(BakeSystems::ResolveIds),
                    (systems::finish_bakes, systems::start_bakes).chain(),
                )
                    .after(TransformSystems::Propagate),
            );
    }

    fn finish(&self, app: &mut App) {
        asset::register(app);
    }
}
