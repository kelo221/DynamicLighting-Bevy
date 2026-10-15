//! Per-frame light packing: baked slots, moved lights and the realtime budget.

use bevy::camera::primitives::{Frustum, Sphere};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use super::budget::{RealtimeCandidate, select_realtime};
use super::buffer::{BakedLightSlots, LightAttachments, PackedLights};
use super::component::DynamicLight;
use super::pack::{LightPose, PackContext, pack};
use crate::effects::LightEffectState;
use crate::shared::{
    BakeId, DynamicGeometryMode, DynamicLightingSettings, GpuLight, GpuLightGlobals,
    ResolvedBakeId, RuntimeQuality,
};

/// How far a baked light may drift from where it was baked before it counts as moved.
const MOVED_DISTANCE: f32 = 1.0e-3;

type LightItem<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Option<&'a LightEffectState>,
    Option<&'a ResolvedBakeId>,
);

/// Packs every visible light into [`PackedLights`]: baked lights at their bake slots, then the
/// realtime lights that fit the budget (lights marked realtime, lights the bake doesn't know,
/// and baked lights that moved).
pub(super) fn pack_lights(
    time: Res<Time>,
    settings: Res<DynamicLightingSettings>,
    slots: Res<BakedLightSlots>,
    attachments: Res<LightAttachments>,
    lights: Query<LightItem>,
    cameras: Query<(&Camera, &GlobalTransform, &Frustum)>,
    mut packed: ResMut<PackedLights>,
) {
    let seconds = time.elapsed_secs();
    let slot_of: HashMap<BakeId, usize> = slots
        .slots
        .iter()
        .enumerate()
        .map(|(index, slot)| (slot.id, index))
        .collect();

    let packed = &mut *packed;
    packed.lights.clear();
    packed.lights.resize(slots.slots.len(), GpuLight::DISABLED);
    packed.baked.clear();
    packed.baked.resize(slots.slots.len(), None);
    packed.realtime.clear();

    let context = |entity: Entity, effect: Option<&LightEffectState>| PackContext {
        seconds,
        effect_intensity: effect.map_or(1.0, LightEffectState::intensity),
        realtime: true,
        bounce_bits_per_pixel: 8,
        cookie_index: attachments.cookies.get(&entity).copied(),
        shadow_cubemap_index: attachments.shadow_cubemaps.get(&entity).copied(),
    };

    let mut candidates = Vec::new();
    for (entity, light, transform, visibility, effect, id) in &lights {
        if !visibility.get() {
            continue;
        }
        let (_, rotation, position) = transform.to_scale_rotation_translation();
        let pose = LightPose { position, rotation };
        let baked_slot = id
            .filter(|_| !light.realtime)
            .and_then(|id| slot_of.get(&id.0).copied())
            .filter(|&slot| slots.slots[slot].origin.distance(position) <= MOVED_DISTANCE);
        if let Some(slot) = baked_slot {
            let context = PackContext {
                realtime: false,
                bounce_bits_per_pixel: slots.slots[slot].bounce_bits_per_pixel,
                ..context(entity, effect)
            };
            packed.lights[slot] = pack(light, pose, context);
            packed.baked[slot] = Some(entity);
        } else {
            candidates.push(RealtimeCandidate {
                key: (entity, pose),
                position,
                radius: light.radius,
            });
        }
    }

    let camera = cameras
        .iter()
        .filter(|(camera, ..)| camera.is_active)
        .max_by_key(|(camera, ..)| camera.order);
    let selected = select_realtime(
        candidates,
        camera.map(|(_, transform, _)| transform.translation()),
        settings.realtime_light_budget as usize,
        |center, radius| {
            camera.is_none_or(|(_, _, frustum)| {
                frustum.intersects_sphere(
                    &Sphere {
                        center: center.into(),
                        radius,
                    },
                    true,
                )
            })
        },
    );
    for candidate in selected {
        let (entity, pose) = candidate.key;
        let Ok((_, light, _, _, effect, _)) = lights.get(entity) else {
            continue;
        };
        packed
            .lights
            .push(pack(light, pose, context(entity, effect)));
        packed.realtime.push(entity);
    }

    packed.globals = GpuLightGlobals {
        ambient: settings.ambient_linear(),
        time: seconds,
        baked_count: slots.slots.len() as u32,
        realtime_count: packed.realtime.len() as u32,
        quality: quality_level(settings.quality),
        flags: if settings.dynamic_geometry == DynamicGeometryMode::LightingOnly {
            GpuLightGlobals::FLAG_LIGHTING_ONLY
        } else {
            0
        } | GpuLightGlobals::debug_flag(settings.debug_view),
    };
}

pub fn quality_level(quality: RuntimeQuality) -> u32 {
    match quality {
        RuntimeQuality::IntegratedGraphics => 0,
        RuntimeQuality::Low => 1,
        RuntimeQuality::Medium => 2,
        RuntimeQuality::High => 3,
    }
}
