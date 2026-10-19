//! Node extras applied as a glTF scene spawns: lights, effects, fog and static flags.
//!
//! [`read_node_extras`] runs in `SpawnScene` after the scene spawner, so a scene's components
//! are in place in the frame it first appears, before anything in `PostUpdate` sees it.

use bevy::gltf::GltfExtras;
use bevy::prelude::*;

use super::GltfImportSettings;
use super::extras;
use crate::bake::BakeStatic;

/// A node's own mesh primitives. Bevy spawns each primitive of a node's mesh as a child that
/// carries the `Mesh3d`; the node itself and its child nodes never carry one.
pub(super) fn own_primitives<'a>(
    children: Option<&'a Children>,
    primitives: &'a Query<(), With<Mesh3d>>,
) -> impl Iterator<Item = Entity> + 'a {
    children
        .into_iter()
        .flatten()
        .copied()
        .filter(|&child| primitives.contains(child))
}

pub(super) fn read_node_extras(
    mut commands: Commands,
    settings: Res<GltfImportSettings>,
    nodes: Query<(Entity, &GltfExtras, Option<&Children>), Added<GltfExtras>>,
    primitives: Query<(), With<Mesh3d>>,
) {
    if !settings.read_node_extras {
        return;
    }
    for (node, extras, children) in &nodes {
        let parsed = extras::parse(&extras.value);
        if parsed.bake_static {
            commands.entity(node).insert(BakeStatic);
            for primitive in own_primitives(children, &primitives) {
                commands.entity(primitive).insert(BakeStatic);
            }
        }
        let mut node = commands.entity(node);
        if let Some(light) = parsed.light {
            node.insert(light);
        }
        if let Some(effect) = parsed.effect {
            node.insert(effect);
        }
        if let Some(fog) = parsed.fog {
            node.insert(fog);
        }
    }
}
