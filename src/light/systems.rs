//! Per-frame light packing: baked slots, moved lights and the realtime budget.

use bevy::camera::primitives::{Frustum, Sphere};
use bevy::prelude::*;

use super::budget::{RealtimeCandidate, select_realtime};
use super::buffer::{BakedLightSlots, LightAttachments, PackedLights, RealtimeViewer};
use super::component::DynamicLight;
use super::pack::{PackContext, pack};
use super::placement::{LightPlacement, pose_of};
use crate::effects::LightEffectState;
use crate::shared::{
    DynamicGeometryMode, DynamicLightingSettings, GpuLight, GpuLightGlobals, RuntimeQuality,
    SkyIrradiance, SunShadowBlock,
};

type LightItem<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Option<&'a LightEffectState>,
);

/// Packs every visible light into [`PackedLights`]: baked lights at their bake slots, then the
/// realtime lights (lights marked realtime, lights the bake doesn't know, and baked lights that
/// moved): distant lights first, all of them, then the others that fit the budget.
#[allow(clippy::too_many_arguments)]
pub(super) fn pack_lights(
    time: Res<Time>,
    settings: Res<DynamicLightingSettings>,
    slots: Res<BakedLightSlots>,
    placement: Res<LightPlacement>,
    attachments: Res<LightAttachments>,
    (sky, sun_shadow): (Res<SkyIrradiance>, Res<SunShadowBlock>),
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
        shadow_bits_per_texel: 1,
        cookie_index: attachments.cookies.get(&entity).copied(),
        // A sun's flag means it has the cascades; it has no slot.
        shadow_cubemap_index: attachments
            .shadow_cubemaps
            .get(&entity)
            .copied()
            .or((attachments.sun_cascades == Some(entity)).then_some(0)),
    };

    let mut candidates = Vec::new();
    let mut distant = Vec::new();
    for (entity, light, transform, visibility, effect) in &lights {
        if !visibility.get() {
            continue;
        }
        let pose = pose_of(transform);
        if let Some(slot) = placement.slot_of(entity) {
            let baked = &slots.slots[slot];
            let context = PackContext {
                realtime: false,
                bounce_bits_per_pixel: baked.bounce_bits_per_pixel,
                shadow_bits_per_texel: baked.shadow_bits_per_texel,
                ..context(entity, effect)
            };
            packed.lights[slot] = pack(light, pose, context);
            packed.baked[slot] = Some(entity);
        } else if light.light_type.is_distant() {
            distant.push((entity, light, pose, effect));
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
    packed.viewer = camera.map(|(camera, transform, _)| RealtimeViewer {
        view_from_world: transform.to_matrix().inverse(),
        clip_from_view: camera.clip_from_view(),
    });
    // Distant lights reach every fragment, so they need no budget, frustum test or grid.
    let realtime_distant = distant.len() as u32;
    for (entity, light, pose, effect) in distant {
        packed
            .lights
            .push(pack(light, pose, context(entity, effect)));
        packed.realtime.push(entity);
    }
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
        baked_distant: slots.distant_count() as u32,
        realtime_distant,
        _padding: [0; 2],
    };
    packed.sky = sky.0;
    packed.sun_shadow = sun_shadow.0;
}

pub fn quality_level(quality: RuntimeQuality) -> u32 {
    quality.level()
}
