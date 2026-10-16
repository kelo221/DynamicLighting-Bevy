//! Per-frame light packing: baked slots, moved lights and the realtime budget.

use bevy::camera::primitives::{Frustum, Sphere};
use bevy::prelude::*;

use super::budget::{RealtimeCandidate, select_realtime};
use super::buffer::{BakedLightSlots, LightAttachments, PackedLights};
use super::component::DynamicLight;
use super::pack::{PackContext, pack};
use super::placement::{LightPlacement, pose_of};
use crate::effects::LightEffectState;
use crate::shared::{
    DynamicGeometryMode, DynamicLightingSettings, GpuLight, GpuLightGlobals, RuntimeQuality,
};

type LightItem<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Option<&'a LightEffectState>,
);

/// Packs every visible light into [`PackedLights`]: baked lights at their bake slots, then the
/// realtime lights that fit the budget (lights marked realtime, lights the bake doesn't know,
/// and baked lights that moved).
#[allow(clippy::too_many_arguments)]
pub(super) fn pack_lights(
    time: Res<Time>,
    settings: Res<DynamicLightingSettings>,
    slots: Res<BakedLightSlots>,
    placement: Res<LightPlacement>,
    attachments: Res<LightAttachments>,
    lights: Query<LightItem>,
    cameras: Query<(&Camera, &GlobalTransform, &Frustum)>,
    mut packed: ResMut<PackedLights>,
) {
    let seconds = time.elapsed_secs();

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
        point_offset: 0.0,
        share: 1.0,
    };

    let mut candidates = Vec::new();
    for (entity, light, transform, visibility, effect) in &lights {
        if !visibility.get() {
            continue;
        }
        let pose = pose_of(transform);
        if let Some(points) = placement.points_of(entity) {
            // An area light baked from several points fills one slot per point.
            let share = 1.0 / points.len() as f32;
            for point in points {
                let context = PackContext {
                    realtime: false,
                    bounce_bits_per_pixel: slots.slots[point.slot].bounce_bits_per_pixel,
                    point_offset: point.offset,
                    share,
                    ..context(entity, effect)
                };
                packed.lights[point.slot] = pack(light, pose, context);
                packed.baked[point.slot] = Some(entity);
            }
        } else {
            candidates.push(RealtimeCandidate {
                key: (entity, pose),
                position: pose.position,
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
        let Ok((_, light, _, _, effect)) = lights.get(entity) else {
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
