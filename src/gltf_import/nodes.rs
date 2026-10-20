//! Node extras applied as a glTF scene spawns: lights, effects, fog, static and shadow flags, and
//! room and portal helpers.
//!
//! [`read_node_extras`] runs in `SpawnScene` after the scene spawner, so a scene's components
//! are in place in the frame it first appears, before anything in `PostUpdate` sees it.

use bevy::ecs::name::NameOrEntity;
use bevy::gltf::GltfExtras;
use bevy::prelude::*;

use super::GltfImportSettings;
use super::extras::{self, Helper};
use super::helpers::helper_mesh_fits;
use crate::bake::BakeStatic;
use crate::portals::{ClosedPortal, Portal, Room};
use crate::shared::NoShadowCasting;

/// A node's own mesh primitives. Bevy spawns each primitive of a node's mesh as a child that
/// carries the `Mesh3d`; the node itself and its child nodes never carry one.
pub(super) fn own_primitives<'a>(
    children: Option<&'a Children>,
    primitives: &'a Query<&Mesh3d>,
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
    nodes: Query<(NameOrEntity, &GltfExtras, Option<&Children>), Added<GltfExtras>>,
    primitives: Query<&Mesh3d>,
    meshes: Option<Res<Assets<Mesh>>>,
) {
    if !settings.read_node_extras {
        return;
    }
    for (label, extras, children) in &nodes {
        let node = label.entity;
        let parsed = extras::parse(&extras.value);
        for warning in &parsed.warnings {
            warn!("dynlight gltf: node {label}: {warning}");
        }
        if let Some(helper) = parsed.helper {
            let own: Vec<Entity> = own_primitives(children, &primitives).collect();
            let bounds = meshes
                .as_deref()
                .and_then(|meshes| mesh_bounds(&own, &primitives, meshes));
            if let Some((min, max)) = bounds
                && !helper_mesh_fits(helper, min, max)
            {
                let shape = match helper {
                    Helper::Room => "unit cube",
                    Helper::Portal { .. } => "±1 square in local XY",
                };
                warn!(
                    "dynlight gltf: node {label}: its mesh spans {min} to {max}, not the \
                     {shape}; the helper is read from the node's transform, so keep the mesh at \
                     ±1 and size it with the object's scale"
                );
            }
            for primitive in own {
                commands.entity(primitive).despawn();
            }
            let mut node = commands.entity(node);
            match helper {
                Helper::Room => {
                    node.insert(Room {
                        half_size: Vec3::ONE,
                    });
                }
                Helper::Portal { closed } => {
                    node.insert(Portal::new(Vec2::splat(2.0)));
                    if closed {
                        node.insert(ClosedPortal);
                    }
                }
            }
            continue;
        }
        if parsed.bake_static {
            commands.entity(node).insert(BakeStatic);
            for primitive in own_primitives(children, &primitives) {
                commands.entity(primitive).insert(BakeStatic);
            }
        }
        if parsed.no_shadow_casting {
            commands.entity(node).insert(NoShadowCasting);
            for primitive in own_primitives(children, &primitives) {
                commands.entity(primitive).insert(NoShadowCasting);
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

/// The local bounds of all `primitives`' vertices, when every mesh is loaded with its positions.
fn mesh_bounds(
    primitives: &[Entity],
    handles: &Query<&Mesh3d>,
    meshes: &Assets<Mesh>,
) -> Option<(Vec3, Vec3)> {
    let mut bounds: Option<(Vec3, Vec3)> = None;
    for &primitive in primitives {
        let mesh = meshes.get(&handles.get(primitive).ok()?.0)?;
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?.as_float3()?;
        for &position in positions {
            let position = Vec3::from_array(position);
            bounds = Some(match bounds {
                Some((min, max)) => (min.min(position), max.max(position)),
                None => (position, position),
            });
        }
    }
    bounds
}
