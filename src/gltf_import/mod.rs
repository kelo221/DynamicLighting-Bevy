//! glTF scenes: lights and static flags from node extras (see [`extras`] for the keys), and
//! `StandardMaterial`s converted to [`DynLitMaterial`](crate::surface::DynLitMaterial).
//!
//! Load glTF files with `GltfLoaderSettings { load_lights: false, .. }` so Bevy never creates
//! its own light components from `KHR_lights_punctual`.

mod convert;
pub mod extras;
mod nodes;

#[cfg(test)]
mod node_tests;

use bevy::app::{SceneSpawnerSystems, SpawnScene};
use bevy::prelude::*;

pub use convert::{GltfImportSettings, dynlit_from_standard};

use crate::surface::DynLitMaterial;

pub struct GltfImportPlugin;

impl Plugin for GltfImportPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<GltfImportSettings>()
            .init_resource::<GltfImportSettings>()
            .init_resource::<convert::Conversions>()
            .add_systems(
                SpawnScene,
                nodes::read_node_extras
                    .after(SceneSpawnerSystems::WorldInstanceSpawn)
                    .after(SceneSpawnerSystems::SceneSpawn),
            );
    }

    fn finish(&self, app: &mut App) {
        let world = app.world();
        if world.contains_resource::<Assets<StandardMaterial>>()
            && world.contains_resource::<Assets<DynLitMaterial>>()
        {
            app.add_systems(PostUpdate, convert::convert_materials);
        }
    }
}
