//! Whether a light uses its baked data this frame: the bake knows it, it stands where it was
//! baked, its radius hasn't grown past the baked one and, for area lights, it keeps the shape it
//! was baked with. Point-like lights may turn freely. A sun has no position or radius: it keeps
//! its bake while it shines the same way with the same disk. Packing and the shadow cubemap pass
//! both read this.

use bevy::ecs::entity::EntityHashMap;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::buffer::{BakedLightSlots, BakedSlot};
use super::component::DynamicLight;
use super::pack::LightPose;
use super::shape::LightShape;
use crate::shared::{BakeId, ResolvedBakeId};

/// How far a baked light may drift from where it was baked before it counts as moved.
const MOVED_DISTANCE: f32 = 1.0e-3;

/// On lights that use their baked shadows this frame. A light loses it when it is marked
/// realtime, the active bake doesn't know it, it moved away from where it was baked, its radius
/// grew past the baked one, or its area light shape changed.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BakedInPlace;

/// The light's pose, read the same way the bake reads it.
pub(super) fn pose_of(transform: &GlobalTransform) -> LightPose {
    let (_, rotation, position) = transform.to_scale_rotation_translation();
    LightPose { position, rotation }
}

/// The active bake's slots by light id.
#[derive(Default)]
pub(super) struct SlotIndex {
    by_id: HashMap<BakeId, usize>,
}

impl SlotIndex {
    pub(super) fn new(slots: &BakedLightSlots) -> Self {
        let by_id = slots
            .slots
            .iter()
            .enumerate()
            .map(|(index, slot)| (slot.id, index))
            .collect();
        Self { by_id }
    }

    /// The slot a light at `pose` uses, if the bake knows it and it still matches its bake.
    pub(super) fn baked_slot(
        &self,
        slots: &BakedLightSlots,
        light: &DynamicLight,
        id: Option<&ResolvedBakeId>,
        pose: LightPose,
    ) -> Option<usize> {
        let id = id.filter(|_| !light.realtime)?;
        self.by_id
            .get(&id.0)
            .copied()
            .filter(|&slot| still_baked(&slots.slots[slot], light, pose))
    }
}

/// Whether `light` at `pose` still matches what `slot` was baked for. A smaller radius still
/// matches: the baked data covers more than the light now reaches.
fn still_baked(slot: &BakedSlot, light: &DynamicLight, pose: LightPose) -> bool {
    let shape = LightShape::of(light, pose.rotation);
    if !slot.shape.matches(&shape) {
        return false;
    }
    if shape.is_distant() {
        return true;
    }
    slot.origin.distance(pose.position) <= MOVED_DISTANCE
        && light.radius <= slot.radius + MOVED_DISTANCE
}

type PlacedLight<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    Option<&'a ResolvedBakeId>,
    Has<BakedInPlace>,
);

/// This frame's baked slot of every light in place, for packing.
#[derive(Resource, Default)]
pub(super) struct LightPlacement {
    /// Rebuilt when the bake changes.
    index: SlotIndex,
    slots: EntityHashMap<usize>,
}

impl LightPlacement {
    /// The baked slot of `light`, if it is baked in place.
    pub(super) fn slot_of(&self, light: Entity) -> Option<usize> {
        self.slots.get(&light).copied()
    }
}

/// Finds the baked slot of every light in place, and adds or removes [`BakedInPlace`] where
/// that changed.
pub(super) fn mark_baked_in_place(
    mut commands: Commands,
    slots: Res<BakedLightSlots>,
    mut placement: ResMut<LightPlacement>,
    lights: Query<PlacedLight>,
) {
    let placement = &mut *placement;
    if slots.is_changed() {
        placement.index = SlotIndex::new(&slots);
    }
    placement.slots.clear();
    for (entity, light, transform, id, marked) in &lights {
        let slot = placement
            .index
            .baked_slot(&slots, light, id, pose_of(transform));
        if let Some(slot) = slot {
            placement.slots.insert(entity, slot);
        }
        let in_place = slot.is_some();
        if in_place && !marked {
            commands.entity(entity).insert(BakedInPlace);
        } else if !in_place && marked {
            commands.entity(entity).remove::<BakedInPlace>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::light::LightType;

    fn pose(position: Vec3, rotation: Quat) -> LightPose {
        LightPose { position, rotation }
    }

    /// `light` baked at `at` in slot 1, after an unrelated light.
    fn baked(light: &DynamicLight, at: LightPose) -> BakedLightSlots {
        let shape = LightShape::of(light, at.rotation);
        let slot = |id, origin, shape: LightShape| BakedSlot {
            id,
            origin,
            radius: light.radius,
            shape,
            bounce_bits_per_pixel: 8,
            shadow_bits_per_texel: if shape == LightShape::Point { 1 } else { 4 },
        };
        BakedLightSlots {
            slots: vec![
                slot(BakeId(99), Vec3::ZERO, LightShape::Point),
                slot(BakeId(1), at.position, shape),
            ],
        }
    }

    fn slot_used(
        slots: &BakedLightSlots,
        light: &DynamicLight,
        id: Option<BakeId>,
        at: LightPose,
    ) -> Option<usize> {
        let id = id.map(ResolvedBakeId);
        SlotIndex::new(slots).baked_slot(slots, light, id.as_ref(), at)
    }

    #[test]
    fn point_lights_stay_baked_until_they_move() {
        let light = DynamicLight::point();
        let home = pose(Vec3::Y, Quat::IDENTITY);
        let slots = baked(&light, home);
        let used = |light: &DynamicLight, id, at| slot_used(&slots, light, id, at);
        assert_eq!(used(&light, Some(BakeId(1)), home), Some(1));
        let nudged = pose(Vec3::Y + Vec3::X * 0.5e-3, Quat::IDENTITY);
        assert_eq!(used(&light, Some(BakeId(1)), nudged), Some(1));
        let turned = pose(Vec3::Y, Quat::from_rotation_z(1.0));
        assert_eq!(used(&light, Some(BakeId(1)), turned), Some(1));
        let moved = pose(Vec3::Y * 1.1, Quat::IDENTITY);
        assert_eq!(used(&light, Some(BakeId(1)), moved), None);
        let realtime = DynamicLight {
            realtime: true,
            ..light.clone()
        };
        assert_eq!(used(&realtime, Some(BakeId(1)), home), None);
        assert_eq!(used(&light, None, home), None);
        let panel = DynamicLight::rectangle(1.0, 0.5);
        assert_eq!(used(&panel, Some(BakeId(1)), home), None, "now a panel");
    }

    #[test]
    fn area_lights_stay_baked_only_with_their_shape() {
        let panel = DynamicLight::rectangle(1.0, 0.5);
        let home = pose(Vec3::Y, Quat::from_rotation_x(-0.5));
        let slots = baked(&panel, home);
        let used = |light: &DynamicLight, at| slot_used(&slots, light, Some(BakeId(1)), at);
        assert_eq!(used(&panel, home), Some(1));
        assert_eq!(
            used(&panel, pose(Vec3::Y * 1.1, home.rotation)),
            None,
            "moved"
        );
        let rotated = pose(Vec3::Y, home.rotation * Quat::from_rotation_y(0.05));
        assert_eq!(used(&panel, rotated), None, "rotated");
        let resized = DynamicLight {
            size: Vec2::new(1.2, 0.5),
            ..panel.clone()
        };
        assert_eq!(used(&resized, home), None, "resized");
        let retyped = DynamicLight {
            light_type: LightType::Point,
            ..panel.clone()
        };
        assert_eq!(used(&retyped, home), None, "now a point");

        // A tube's height is unused, so changing it keeps the bake.
        let tube = DynamicLight::tube(1.2);
        let tube_slots = baked(&tube, home);
        let thicker = DynamicLight {
            size: Vec2::new(1.2, 0.5),
            ..tube.clone()
        };
        assert_eq!(
            slot_used(&tube_slots, &thicker, Some(BakeId(1)), home),
            Some(1)
        );
    }

    #[test]
    fn suns_stay_baked_wherever_they_are_until_they_turn() {
        let sun = DynamicLight::sun();
        let home = pose(Vec3::Y, Quat::from_rotation_x(-1.0));
        let slots = baked(&sun, home);
        assert_eq!(slots.distant_count(), 1);
        let used = |light: &DynamicLight, at| slot_used(&slots, light, Some(BakeId(1)), at);
        assert_eq!(used(&sun, home), Some(1));
        let moved = pose(Vec3::splat(50.0), home.rotation);
        assert_eq!(used(&sun, moved), Some(1), "a sun has no position");
        let wider = DynamicLight {
            radius: 1.0e6,
            ..sun.clone()
        };
        assert_eq!(used(&wider, home), Some(1), "nor a radius");
        let turned = pose(Vec3::Y, Quat::from_rotation_y(0.1) * home.rotation);
        assert_eq!(used(&sun, turned), None, "turned");
        let softer = DynamicLight {
            angular_diameter: 3.0,
            ..sun.clone()
        };
        assert_eq!(used(&softer, home), None, "a wider disk");
        assert_eq!(used(&DynamicLight::point(), home), None, "now a point");
    }

    #[test]
    fn lights_stay_baked_only_while_their_radius_does_not_grow() {
        let home = pose(Vec3::Y, Quat::from_rotation_x(-0.5));
        for light in [DynamicLight::point(), DynamicLight::rectangle(1.0, 0.5)] {
            let slots = baked(&light, home);
            let used = |radius: f32| {
                let resized = DynamicLight {
                    radius,
                    ..light.clone()
                };
                slot_used(&slots, &resized, Some(BakeId(1)), home)
            };
            assert_eq!(used(light.radius), Some(1));
            assert_eq!(used(light.radius * 0.5), Some(1), "shrunk");
            assert_eq!(used(light.radius + 0.5e-3), Some(1), "within the tolerance");
            assert_eq!(used(light.radius * 1.5), None, "grown");
        }
    }
}
