//! The `.dynlight` asset: a [`BakedScene`] loaded through Bevy's asset server.

use std::fmt;

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

/// Registers the asset and loader when the app has an asset server; headless apps without
/// `AssetPlugin` skip it and can still bake and [`decode`] files directly.
pub(super) fn register(app: &mut App) {
    if app.is_plugin_added::<AssetPlugin>() {
        app.init_asset::<DynLightBake>()
            .register_asset_loader(DynLightBakeLoader);
    }
}
