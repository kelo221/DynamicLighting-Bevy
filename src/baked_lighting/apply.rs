//! Applies the active `.dynlight` bake: receiver records and the aux buffer for the shaders, and
//! the baked light slots for the light buffer.

use bevy::prelude::*;

use crate::bake::{BakedScene, DynLightBake};
use crate::light::{BakedLightSlots, BakedSlot};
use crate::shared::DynamicGeometryMode;

use super::layout::{BakedBuffers, ReceiverInput, SceneInput, flags};
use super::systems::AppliedBake;

/// Bounce bits per pixel for lights without baked bounce; their bounce offsets are all 0.
const NO_BOUNCE_BITS: u32 = 8;

/// The bake the scene is lit with. Point it at a `.dynlight` asset; `None` lights everything
/// dynamically. A bake that changes on disk is applied again when the asset reloads.
#[derive(Resource, Default, Clone, Debug)]
pub struct ActiveBake(pub Option<Handle<DynLightBake>>);

/// The GPU buffers for `scene`.
pub fn scene_buffers(scene: &BakedScene) -> BakedBuffers {
    let mut flag_bits = 0;
    if scene.settings.bounce {
        flag_bits |= flags::BOUNCE_LAYOUT;
    }
    match scene.settings.dynamic_geometry {
        DynamicGeometryMode::LightingOnly => {}
        DynamicGeometryMode::DistanceCubes => flag_bits |= flags::CUBE_BIAS,
        DynamicGeometryMode::Angular => flag_bits |= flags::CUBE_ANGULAR,
    }
    let cube_count = if scene.distance_cubes.is_empty() {
        0
    } else {
        scene.lights.len() as u32
    };
    let meshes = scene.meshes.iter().map(|mesh| ReceiverInput {
        id: mesh.id,
        content_hash: mesh.content_hash,
        resolution: mesh.resolution,
        triangles: &mesh.triangles,
    });
    BakedBuffers::build(
        meshes,
        SceneInput {
            baked_lights: scene.lights.len() as u32,
            bvh: bytemuck::cast_slice(&scene.light_bvh),
            cubes: &scene.distance_cubes,
            cube_count,
            flags: flag_bits,
        },
    )
}

/// The light buffer slots for `scene`: one per baked light, in BVH order.
pub fn scene_slots(scene: &BakedScene) -> Vec<BakedSlot> {
    scene
        .lights
        .iter()
        .map(|light| BakedSlot {
            id: light.id,
            origin: light.origin,
            bounce_bits_per_pixel: light
                .bounce_compression
                .map_or(NO_BOUNCE_BITS, |compression| compression.bits_per_pixel()),
        })
        .collect()
}

/// Applies [`ActiveBake`] when it changes or its asset (re)loads.
pub(super) fn apply_bake(
    active: Res<ActiveBake>,
    bakes: Res<Assets<DynLightBake>>,
    mut events: MessageReader<AssetEvent<DynLightBake>>,
    mut applied: ResMut<AppliedBake>,
    mut slots: ResMut<BakedLightSlots>,
) {
    let wanted = active.0.as_ref().map(Handle::id);
    let reloaded = events.read().any(|event| match event {
        AssetEvent::LoadedWithDependencies { id } | AssetEvent::Modified { id } => {
            Some(*id) == wanted
        }
        _ => false,
    });
    if !active.is_changed() && !reloaded {
        return;
    }
    match wanted.and_then(|id| bakes.get(id)) {
        Some(bake) => {
            applied.buffers = scene_buffers(&bake.scene);
            slots.slots = scene_slots(&bake.scene);
            info!(
                "dynlight: applied bake with {} meshes and {} lights",
                bake.scene.meshes.len(),
                bake.scene.lights.len()
            );
        }
        None => {
            applied.buffers = BakedBuffers::empty();
            slots.slots.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::{BakedLight, BakedMesh, LightBvhNode};
    use crate::baked_lighting::layout::header;
    use crate::light::BounceCompression;
    use crate::shared::BakeId;

    fn scene() -> BakedScene {
        let mut scene = BakedScene::default();
        scene.settings.bounce = true;
        scene.settings.dynamic_geometry = DynamicGeometryMode::DistanceCubes;
        scene.lights = vec![
            BakedLight {
                id: BakeId(1),
                origin: Vec3::X,
                radius: 2.0,
                bounce_compression: Some(BounceCompression::Bits5),
            },
            BakedLight {
                id: BakeId(2),
                origin: Vec3::Y,
                radius: 3.0,
                bounce_compression: None,
            },
        ];
        scene.light_bvh = vec![LightBvhNode {
            aabb_min: [-1.0; 3],
            left_first: 0,
            aabb_max: [1.0; 3],
            count: 2,
        }];
        scene.meshes = vec![BakedMesh {
            id: BakeId(9),
            content_hash: 77,
            resolution: 16,
            triangles: vec![4, 0, 0, 5, 0],
        }];
        scene.distance_cubes = vec![0; 2 * 64 * 64 * 3];
        scene
    }

    #[test]
    fn buffers_carry_the_scene_settings() {
        let buffers = scene_buffers(&scene());
        let aux = &buffers.aux;
        assert_eq!(aux[header::BAKED_LIGHTS], 2);
        assert_eq!(aux[header::BVH_NODE_COUNT], 1);
        assert_eq!(aux[header::CUBE_COUNT], 2);
        assert_eq!(aux[header::FLAGS], flags::BOUNCE_LAYOUT | flags::CUBE_BIAS);
        let receiver = buffers.receivers[&BakeId(9)];
        assert_eq!((receiver.tag, receiver.content_hash), (1, 77));
        assert_eq!(receiver.triangle_count, 1);
    }

    #[test]
    fn slots_follow_bvh_order_with_bounce_precision() {
        let slots = scene_slots(&scene());
        assert_eq!(slots.len(), 2);
        assert_eq!(
            (slots[0].id, slots[0].bounce_bits_per_pixel),
            (BakeId(1), 5)
        );
        assert_eq!(
            (slots[1].id, slots[1].bounce_bits_per_pixel),
            (BakeId(2), 8)
        );
        assert_eq!(slots[1].origin, Vec3::Y);
    }
}
