//! The `.dynlight` asset loader: reads the packed file, inflates and parses it as a stream and
//! builds the GPU buffers on the loader thread.

use std::fmt;

use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use bevy::prelude::*;

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
        // The packed file is read whole with the asset reader's own async reads; blocking on
        // them inside the parser could deadlock a reader that needs this executor. Only the
        // inflate is streamed, so the unpacked payload, the big copy, is never held whole next
        // to the parsed scene.
        let mut packed = Vec::new();
        reader
            .read_to_end(&mut packed)
            .await
            .map_err(DynLightBakeLoaderError::Io)?;
        let mut scene = decode_stream(&packed[..]).map_err(DynLightBakeLoaderError::Format)?;
        drop(packed);
        // The GPU words are made here, on the loader thread; the scene's copy is released as it
        // goes, so one copy of the triangles remains.
        let buffers = Some(take_scene_buffers(&mut scene));
        Ok(DynLightBake { scene, buffers })
    }

    fn extensions(&self) -> &[&str] {
        &["dynlight"]
    }
}
