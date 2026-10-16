//! Whether a light uses its baked data this frame: the bake knows every one of its shadow points
//! and each still stands where it was baked. A point-like light's only point is its position,
//! so turning it keeps its bake; an area light's points spread over its shape, so turning it or
//! resizing it moves them. Packing and the shadow cubemap pass both read this.

use core::ops::Range;

use bevy::ecs::entity::EntityHashMap;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::buffer::{BakedLightSlots, BakedSlot};
use super::component::DynamicLight;
use super::pack::LightPose;
use super::shadow_points::shadow_point_id;
use crate::shared::{BakeId, ChannelFlags, ResolvedBakeId};

/// How far a baked light may drift from where it was baked before it counts as moved.
const MOVED_DISTANCE: f32 = 1.0e-3;

/// On lights that use their baked shadows this frame. A light loses it when it is marked
/// realtime, the active bake doesn't know it, or it moved away from where it was baked.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BakedInPlace;

/// One baked record of a light: its bake slot, and its shadow point's offset from the light's
/// position along the light's local X and Y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BakedPoint {
    pub(super) slot: usize,
    pub(super) offset: Vec2,
}

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

    /// Fills `points` with the `slots` a light at `pose` uses, one per shadow point. Returns
    /// false and leaves `points` empty unless every point is baked where it stands now.
    pub(super) fn baked_points(
        &self,
        slots: &BakedLightSlots,
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
                .filter(|&slot| still_baked(&slots.slots[slot], point.position));
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

/// [`ChannelFlags::group`] for the records of a light baked from `points`: its first slot plus
/// one when it has several points in consecutive slots, so the shaders shade it once; otherwise
/// 0, and each record is shaded on its own (bakes that didn't keep the points together).
pub(super) fn point_group(points: &[BakedPoint]) -> u32 {
    let Some(first) = points.first().map(|point| point.slot) else {
        return 0;
    };
    let consecutive = points
        .iter()
        .enumerate()
        .all(|(k, point)| point.slot == first + k);
    let group = first as u32 + 1;
    if points.len() > 1 && consecutive && group <= ChannelFlags::MAX_GROUP {
        group
    } else {
        0
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

/// This frame's baked records of every light in place, for packing.
#[derive(Resource, Default)]
pub(super) struct LightPlacement {
    /// Rebuilt when the bake changes.
    index: SlotIndex,
    lights: EntityHashMap<Range<usize>>,
    points: Vec<BakedPoint>,
}

impl LightPlacement {
    /// The baked records of `light`, one per shadow point, if it is baked in place.
    pub(super) fn points_of(&self, light: Entity) -> Option<&[BakedPoint]> {
        self.lights
            .get(&light)
            .map(|range| &self.points[range.clone()])
    }
}

/// Finds the baked records of every light in place, and adds or removes [`BakedInPlace`] where
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
    placement.lights.clear();
    placement.points.clear();
    let mut points = Vec::new();
    for (entity, light, transform, id, marked) in &lights {
        let pose = pose_of(transform);
        let in_place = placement
            .index
            .baked_points(&slots, light, id, pose, &mut points);
        if in_place {
            let start = placement.points.len();
            placement.points.extend_from_slice(&points);
            placement
                .lights
                .insert(entity, start..placement.points.len());
        }
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
            .baked_points(slots, light, id.as_ref(), at, &mut points)
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
        // Turning or resizing the panel moves the points off their bake.
        let turned = pose(
            Vec3::Y,
            Quat::from_rotation_x(-1.0) * Quat::from_rotation_z(0.2),
        );
        assert_eq!(used(&panel, turned), None);
        let longer = DynamicLight::rectangle(1.6, 0.4);
        assert_eq!(used(&longer, home), None);
        let taller = DynamicLight::rectangle(1.2, 0.8);
        assert_eq!(used(&taller, home), None);
        // A tube's points lie along its length.
        let tube = DynamicLight::tube(1.2);
        let tube_slots = baked(&tube, BakeId(2), home);
        let thicker = DynamicLight {
            size: Vec2::new(1.2, 0.5),
            ..tube.clone()
        };
        let mut points = Vec::new();
        let id = ResolvedBakeId(BakeId(2));
        assert!(SlotIndex::new(&tube_slots).baked_points(
            &tube_slots,
            &thicker,
            Some(&id),
            home,
            &mut points
        ));
        let fewer = DynamicLight {
            shadow_points: 2,
            ..panel.clone()
        };
        assert_eq!(used(&fewer, home), None);

        let mut partial = slots.clone();
        let points: Vec<BakedPoint> = [4, 5, 6]
            .map(|slot| BakedPoint {
                slot,
                offset: Vec2::ZERO,
            })
            .to_vec();
        assert_eq!(point_group(&points), 5);
        assert_eq!(point_group(&points[..1]), 0, "one point needs no group");
        let scattered = [points[0], points[2], points[1]];
        assert_eq!(point_group(&scattered), 0);
        let far = [4095, 4096].map(|slot| BakedPoint {
            slot,
            offset: Vec2::ZERO,
        });
        assert_eq!(point_group(&far), 0, "past the channel's 12 bits");

        partial.slots.pop();
        assert_eq!(slots_used(&partial, &panel, Some(BakeId(1)), home), None);
    }
}
