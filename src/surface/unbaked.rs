//! Reports lit meshes without bake data, for the `unbaked_meshes` shader feature.

use bevy::prelude::*;

use crate::baked_lighting::BakedReceiver;
use crate::shared::ShaderFeaturesInUse;
use bevy::pbr::Material;

/// `M` is `DynLitMaterial` or an external material (`surface/external.rs`).
pub(super) fn note_unbaked_meshes<M: Material>(
    unbaked: Query<(), (With<MeshMaterial3d<M>>, Without<BakedReceiver>)>,
    mut in_use: ResMut<ShaderFeaturesInUse>,
) {
    if !unbaked.is_empty() {
        in_use.0.unbaked_meshes = true;
    }
}
