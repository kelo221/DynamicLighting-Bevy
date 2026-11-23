//! The `.dynlight` asset: a [`BakedScene`] with its prepared GPU buffers. The loader that fills it
//! from a file lives in `baked_lighting`.

use std::sync::Arc;

use bevy::prelude::*;

use super::format::BakedScene;
use super::prepared::BakedBuffers;

/// A loaded `.dynlight` file.
#[derive(Asset, TypePath, Clone, Debug, PartialEq)]
pub struct DynLightBake {
    pub scene: BakedScene,
    /// The GPU buffers for [`Self::scene`], built by the loader off the main thread. `Some` for
    /// a bake loaded from a file; its scene has given up the meshes' triangle words (they live
    /// on in these buffers). `None` for a bake made in this app, whose buffers are built when
    /// it is applied.
    pub buffers: Option<BakedBuffers>,
}

/// Registers the asset with an asset server, or just the in-memory collection for headless apps
/// without `AssetPlugin`.
pub(super) fn register(app: &mut App) {
    if app.is_plugin_added::<AssetPlugin>() {
        app.init_asset::<DynLightBake>();
    } else {
        app.init_resource::<Assets<DynLightBake>>()
            .init_resource::<HeadlessBakes>()
            .add_message::<AssetEvent<DynLightBake>>()
            .add_systems(Last, free_unused_bakes);
    }
}

/// Without `AssetPlugin` nothing frees an asset once its handles are gone (Bevy's
/// `Assets::track_assets` needs the asset server). Headless apps keep a handle to every bake
/// here, and a bake is removed once that handle is its last.
#[derive(Resource, Default)]
pub(super) struct HeadlessBakes(Vec<Handle<DynLightBake>>);

impl HeadlessBakes {
    pub(super) fn hold(&mut self, handle: &Handle<DynLightBake>) {
        self.0.push(handle.clone());
    }
}

fn free_unused_bakes(mut held: ResMut<HeadlessBakes>, mut assets: ResMut<Assets<DynLightBake>>) {
    held.0.retain(|handle| {
        let Handle::Strong(strong) = handle else {
            return false;
        };
        if Arc::strong_count(strong) > 1 {
            return true;
        }
        assets.remove(handle);
        false
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_bakes_are_freed_with_their_last_handle() {
        let mut app = App::new();
        register(&mut app);
        let world = app.world_mut();
        let bake = DynLightBake {
            scene: BakedScene::default(),
            buffers: None,
        };
        let handle = world.resource_mut::<Assets<DynLightBake>>().add(bake);
        world.resource_mut::<HeadlessBakes>().hold(&handle);
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Assets<DynLightBake>>().len(), 1);
        drop(handle);
        app.update();
        assert!(app.world().resource::<Assets<DynLightBake>>().is_empty());
        assert!(app.world().resource::<HeadlessBakes>().0.is_empty());
    }
}
