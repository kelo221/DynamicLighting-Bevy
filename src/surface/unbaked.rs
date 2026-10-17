//! Reports lit meshes without bake data, for the `unbaked_meshes` shader feature.

use bevy::prelude::*;

use super::material::DynLitMaterial;
use crate::baked_lighting::BakedReceiver;
use crate::shared::ShaderFeaturesInUse;

pub(super) fn note_unbaked_meshes(
    unbaked: Query<(), (With<MeshMaterial3d<DynLitMaterial>>, Without<BakedReceiver>)>,
    mut in_use: ResMut<ShaderFeaturesInUse>,
) {
    if !unbaked.is_empty() {
        in_use.0.unbaked_meshes = true;
    }
}
