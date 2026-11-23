//! The `.dynlight` asset loader: streams the file through the format reader and builds the GPU
//! buffers on the loader thread.

use std::fmt;

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::AsyncReadExt};

use crate::bake::{DynLightBake, FormatError, decode_stream};

use super::take_scene_buffers;

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
