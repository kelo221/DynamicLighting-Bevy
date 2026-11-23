//! The `.dynlight` asset: a [`BakedScene`] loaded through Bevy's asset server.

use std::fmt;
use std::sync::Arc;

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::AsyncReadExt};

use crate::baked_lighting::{BakedBuffers, take_scene_buffers};

use super::format::{BakedScene, FormatError, decode_stream};

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

/// Lets the parser pull from the asset reader. The loader task is the only thing waiting on it.
struct BlockingReader<'a>(&'a mut dyn Reader);

impl std::io::Read for BlockingReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        block_on(self.0.read(buf))
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
        // Inflated and parsed as it is read: the packed file and the unpacked payload are never
        // held whole next to the parsed scene.
        let mut scene =
            decode_stream(BlockingReader(reader)).map_err(DynLightBakeLoaderError::Format)?;
        // The GPU words are made here, on the loader thread; the scene's copy is released as it
        // goes, so one copy of the triangles remains.
        let buffers = Some(take_scene_buffers(&mut scene));
        Ok(DynLightBake { scene, buffers })
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
