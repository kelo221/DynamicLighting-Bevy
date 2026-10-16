//! The `.dynlight` asset: a [`BakedScene`] loaded through Bevy's asset server.

use std::fmt;
use std::sync::Arc;

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;

use super::format::{BakedScene, FormatError, decode};

/// A loaded `.dynlight` file.
#[derive(Asset, TypePath, Clone, Debug, PartialEq)]
pub struct DynLightBake {
    pub scene: BakedScene,
}

/// Loads `.dynlight` files into [`DynLightBake`] assets.
#[derive(Default, TypePath)]
pub struct DynLightBakeLoader;

/// Why a `.dynlight` asset failed to load.
#[derive(Debug)]
pub enum DynLightBakeLoaderError {
    Io(std::io::Error),
    Format(FormatError),
}

impl fmt::Display for DynLightBakeLoaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "couldn't read the .dynlight file: {error}"),
            Self::Format(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for DynLightBakeLoaderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Format(error) => Some(error),
        }
    }
}

impl AssetLoader for DynLightBakeLoader {
    type Asset = DynLightBake;
    type Settings = ();
    type Error = DynLightBakeLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .await
            .map_err(DynLightBakeLoaderError::Io)?;
        let scene = decode(&bytes).map_err(DynLightBakeLoaderError::Format)?;
        Ok(DynLightBake { scene })
    }

    fn extensions(&self) -> &[&str] {
        &["dynlight"]
    }
}

/// Registers the loader with an asset server, or just the in-memory asset collection for
/// headless apps without `AssetPlugin`.
pub(super) fn register(app: &mut App) {
    if app.is_plugin_added::<AssetPlugin>() {
        app.init_asset::<DynLightBake>()
            .register_asset_loader(DynLightBakeLoader);
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
