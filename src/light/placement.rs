//! Whether a light uses its baked data this frame: the bake knows every one of its shadow points
//! and each still stands where it was baked. A point-like light's only point is its position,
//! so turning it keeps its bake; an area light's points lie along its long side, so turning it
//! or changing its length moves them. Packing and the shadow cubemap pass both read this.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::buffer::{BakedLightSlots, BakedSlot};
use super::component::DynamicLight;
use super::pack::LightPose;
use super::shadow_points::shadow_point_id;
use crate::shared::{BakeId, ResolvedBakeId};

/// How far a baked light may drift from where it was baked before it counts as moved.
const MOVED_DISTANCE: f32 = 1.0e-3;

/// On lights that use their baked shadows this frame. A light loses it when it is marked
/// realtime, the active bake doesn't know it, or it moved away from where it was baked.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BakedInPlace;

/// One baked record of a light: its bake slot, and its shadow point's distance from the light's
/// position along the long axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BakedPoint {
    pub(super) slot: usize,
    pub(super) offset: f32,
}

/// The light's pose, read the same way the bake reads it.
pub(super) fn pose_of(transform: &GlobalTransform) -> LightPose {
    let (_, rotation, position) = transform.to_scale_rotation_translation();
    LightPose { position, rotation }
}

/// The active bake's slots by light id.
pub(super) struct SlotIndex<'a> {
    slots: &'a BakedLightSlots,
    by_id: HashMap<BakeId, usize>,
}

impl<'a> SlotIndex<'a> {
    pub(super) fn new(slots: &'a BakedLightSlots) -> Self {
        let by_id = slots
            .slots
            .iter()
            .enumerate()
            .map(|(index, slot)| (slot.id, index))
            .collect();
        Self { slots, by_id }
    }

    /// Fills `points` with the slots a light at `pose` uses, one per shadow point. Returns false
    /// and leaves `points` empty unless every point is baked where it stands now.
    pub(super) fn baked_points(
        &self,
        light: &DynamicLight,
        id: Option<&ResolvedBakeId>,
        pose: LightPose,
        points: &mut Vec<BakedPoint>,
    ) -> bool {
        points.clear();
        let Some(id) = id.filter(|_| !light.realtime) else {
            return false;
        };
        let count = light.shadow_point_count();
        for point in light.shadow_point_layout(pose.position, pose.rotation) {
            let slot = self
                .by_id
                .get(&shadow_point_id(id.0, point.index, count))
                .copied()
                .filter(|&slot| still_baked(&self.slots.slots[slot], point.position));
            let Some(slot) = slot else {
                points.clear();
                return false;
            };
            points.push(BakedPoint {
                slot,
                offset: point.offset,
            });
        }
        true
    }
}

/// Whether a shadow point at `position` still stands where `slot` was baked.
fn still_baked(slot: &BakedSlot, position: Vec3) -> bool {
    slot.origin.distance(position) <= MOVED_DISTANCE
}

type PlacedLight<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    Option<&'a ResolvedBakeId>,
    Has<BakedInPlace>,
);

/// Adds or removes [`BakedInPlace`] where it changed.
pub(super) fn mark_baked_in_place(
    mut commands: Commands,
    slots: Res<BakedLightSlots>,
    lights: Query<PlacedLight>,
) {
    let index = SlotIndex::new(&slots);
    let mut points = Vec::new();
    for (entity, light, transform, id, marked) in &lights {
        let in_place = index.baked_points(light, id, pose_of(transform), &mut points);
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

    fn slot(id: BakeId, origin: Vec3) -> BakedSlot {
        BakedSlot {
            id,
            origin,
            bounce_bits_per_pixel: 8,
        }
    }

    fn pose(position: Vec3, rotation: Quat) -> LightPose {
        LightPose { position, rotation }
    }

    /// The slots of `light` baked at `at`, plus an unrelated light.
    fn baked(light: &DynamicLight, id: BakeId, at: LightPose) -> BakedLightSlots {
        let count = light.shadow_point_count();
        let mut slots = vec![slot(BakeId(99), Vec3::ZERO)];
        slots.extend(
            light
                .shadow_point_layout(at.position, at.rotation)
                .map(|p| slot(shadow_point_id(id, p.index, count), p.position)),
        );
        BakedLightSlots { slots }
    }

    fn slots_used(
        slots: &BakedLightSlots,
        light: &DynamicLight,
        id: Option<BakeId>,
        at: LightPose,
    ) -> Option<Vec<usize>> {
        let mut points = Vec::new();
        let id = id.map(ResolvedBakeId);
        SlotIndex::new(slots)
            .baked_points(light, id.as_ref(), at, &mut points)
            .then(|| points.iter().map(|p| p.slot).collect())
    }

    #[test]
    fn point_lights_stay_baked_until_they_move() {
        let light = DynamicLight::point();
        let home = pose(Vec3::Y, Quat::IDENTITY);
        let slots = baked(&light, BakeId(1), home);
        let used = |light: &DynamicLight, id, at| slots_used(&slots, light, id, at);
        assert_eq!(used(&light, Some(BakeId(1)), home), Some(vec![1]));
        let nudged = pose(Vec3::Y + Vec3::X * 0.5e-3, Quat::IDENTITY);
        assert_eq!(used(&light, Some(BakeId(1)), nudged), Some(vec![1]));
        let turned = pose(Vec3::Y, Quat::from_rotation_z(1.0));
        assert_eq!(used(&light, Some(BakeId(1)), turned), Some(vec![1]));
        let moved = pose(Vec3::Y * 1.1, Quat::IDENTITY);
        assert_eq!(used(&light, Some(BakeId(1)), moved), None);
        let realtime = DynamicLight {
            realtime: true,
            ..light.clone()
        };
        assert_eq!(used(&realtime, Some(BakeId(1)), home), None);
        assert_eq!(used(&light, None, home), None);
    }

    #[test]
    fn area_lights_need_every_shadow_point_in_place() {
        let panel = DynamicLight::rectangle(1.2, 0.4);
        let home = pose(Vec3::Y, Quat::from_rotation_x(-1.0));
        let slots = baked(&panel, BakeId(1), home);
        let used = |light: &DynamicLight, at| slots_used(&slots, light, Some(BakeId(1)), at);
        assert_eq!(used(&panel, home), Some(vec![1, 2, 3]));
        // Turning the panel or changing its length moves the points off their bake.
        let turned = pose(
            Vec3::Y,
            Quat::from_rotation_x(-1.0) * Quat::from_rotation_z(0.2),
        );
        assert_eq!(used(&panel, turned), None);
        let longer = DynamicLight::rectangle(1.6, 0.4);
        assert_eq!(used(&longer, home), None);
        let taller = DynamicLight::rectangle(1.2, 0.8);
        assert_eq!(used(&taller, home), Some(vec![1, 2, 3]));
        let fewer = DynamicLight {
            shadow_points: 2,
            ..panel.clone()
        };
        assert_eq!(used(&fewer, home), None);

        let mut partial = slots.clone();
        partial.slots.pop();
        assert_eq!(slots_used(&partial, &panel, Some(BakeId(1)), home), None);
    }
}
