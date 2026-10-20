//! Meshes that cast no shadows.

use bevy::prelude::*;

/// Keeps a mesh out of every shadow: the bake traces through it (shadows, bounce, distance
/// cubes, sun maps, sky probes) and realtime shadow cubemaps and cascades don't draw it. The mesh
/// is still lit, and a [`BakeStatic`](crate::bake::BakeStatic) mesh still receives baked light.
/// Changing it on a baked mesh leaves the shadows it cast in the bake until the next bake.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[reflect(Component, Default, Debug, PartialEq, Clone)]
pub struct NoShadowCasting;
