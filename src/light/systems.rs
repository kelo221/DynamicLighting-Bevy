//! Per-frame light packing: the sky choice, baked slots, moved lights and the realtime budget.

use bevy::camera::primitives::Frustum;
use bevy::prelude::*;

use super::budget::{RealtimeCandidate, select_realtime};
use super::buffer::{
    BakedLightSlots, LightAttachments, LightChoices, PackedLights, RealtimeViewer,
};
use super::choice::{LightRank, first_ranked};
use super::component::DynamicLight;
use super::kinds::LightType;
use super::pack::{PackContext, pack};
use super::placement::{LightPlacement, pose_of};
use super::viewer::{CaptureViewpoint, LightingViewer, viewer_rank};
use crate::effects::LightEffectState;
use crate::portals::{self, RoomGraph, RoomView};
use crate::shared::{
    DynamicGeometryMode, DynamicLightingSettings, GpuLight, GpuLightGlobals, NamePaths,
    ReflectionProbeRecords, RuntimeQuality, SkyIrradiance, SunShadowBlock,
};

type CameraItem<'a> = (
    &'a Camera,
    &'a GlobalTransform,
    &'a Frustum,
    Has<Camera3d>,
    Has<LightingViewer>,
    Option<&'a RoomView>,
);

type SkyItem<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
);

/// Chooses the sky light that lights the scene into [`LightChoices`]: the first by
/// [`LightRank`] among the visible, valid sky lights.
pub(super) fn choose_sky(
    lights: Query<SkyItem>,
    paths: NamePaths,
    mut choices: ResMut<LightChoices>,
    mut warned: Local<bool>,
) {
    let skies: Vec<_> = lights
        .iter()
        .filter(|(_, light, _, visible)| {
            light.light_type == LightType::Sky && light.is_valid() && visible.get()
        })
        .collect();
    let sky = match skies.as_slice() {
        [] => None,
        [(entity, ..)] => Some(*entity),
        _ => {
            let ranked = skies.iter().map(|&(entity, light, transform, _)| {
                (
                    entity,
                    LightRank::new(light, pose_of(transform), paths.name_path(entity)),
                )
            });
            let sky = first_ranked(ranked);
            if !*warned && let Some(sky) = sky {
                *warned = true;
                warn!(
                    "dynlight sky: {} sky lights; only one lights the scene, \"{}\"",
                    skies.len(),
                    paths.name_path(sky).join("/")
                );
            }
            sky
        }
    };
    if choices.sky != sky {
        choices.sky = sky;
    }
}

type LightItem<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Option<&'a LightEffectState>,
);

/// Packs every visible light into [`PackedLights`]: baked lights at their bake slots, then the
/// realtime lights (lights marked realtime, lights the bake doesn't know, and baked lights that
/// moved): distant lights first, all of them in [`LightRank`] order, then the others that fit
/// the budget. Of the sky lights only the chosen one is packed.
///
/// A realtime light within the budget must lie in the viewer's frustum and be seen by some
/// relevant camera through the rooms. While a [`CaptureViewpoint`] is set they are chosen around
/// it instead, in every direction.
#[allow(clippy::too_many_arguments)]
pub(super) fn pack_lights(
    time: Res<Time>,
    settings: Res<DynamicLightingSettings>,
    slots: Res<BakedLightSlots>,
    placement: Res<LightPlacement>,
    (attachments, choices): (Res<LightAttachments>, Res<LightChoices>),
    (sky, sun_shadow, probes): (
        Res<SkyIrradiance>,
        Res<SunShadowBlock>,
        Res<ReflectionProbeRecords>,
    ),
    lights: Query<LightItem>,
    (cameras, rooms, viewpoint): (Query<CameraItem>, Res<RoomGraph>, Res<CaptureViewpoint>),
    paths: NamePaths,
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
        colored_bounce: false,
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
        if !visibility.get() || (light.light_type == LightType::Sky && choices.sky != Some(entity))
        {
            continue;
        }
        let pose = pose_of(transform);
        if let Some(slot) = placement.slot_of(entity) {
            let baked = &slots.slots[slot];
            let context = PackContext {
                realtime: false,
                bounce_bits_per_pixel: baked.bounce_bits_per_pixel,
                colored_bounce: baked.colored_bounce,
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

    let relevant: Vec<_> = cameras
        .iter()
        .filter_map(|(camera, transform, frustum, three_d, marked, view)| {
            let rank = viewer_rank(camera.is_active, camera.order, three_d, marked)?;
            Some((rank, (camera, transform, frustum), view))
        })
        .collect();
    let camera = relevant
        .iter()
        .max_by_key(|(rank, ..)| *rank)
        .map(|&(_, camera, _)| camera)
        .filter(|_| viewpoint.0.is_none());
    packed.viewer = camera.map(|(camera, transform, _)| RealtimeViewer {
        view_from_world: transform.to_matrix().inverse(),
        clip_from_view: camera.clip_from_view(),
    });
    // Distant lights reach every fragment, so they need no budget, frustum test or grid.
    if distant.len() > 1 {
        let mut ranked: Vec<_> = distant
            .drain(..)
            .map(|item| {
                (
                    LightRank::new(item.1, item.2, paths.name_path(item.0)),
                    item,
                )
            })
            .collect();
        ranked.sort_by(|(a, _), (b, _)| a.order(b));
        distant.extend(ranked.into_iter().map(|(_, item)| item));
    }
    let realtime_distant = distant.len() as u32;
    for (entity, light, pose, effect) in distant {
        packed
            .lights
            .push(pack(light, pose, context(entity, effect)));
        packed.realtime.push(entity);
    }
    let selected = select_realtime(
        candidates,
        viewpoint
            .0
            .or(camera.map(|(_, transform, _)| transform.translation())),
        settings.realtime_light_budget as usize,
        |center, radius| {
            // The viewer's frustum alone bounds the realtime lights, as it bounds the clusters.
            camera.is_none_or(|(_, _, frustum)| {
                portals::sees(&rooms, [(frustum, None)], center, radius)
                    && portals::sees(
                        &rooms,
                        relevant
                            .iter()
                            .map(|&(_, (_, _, frustum), view)| (frustum, view)),
                        center,
                        radius,
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
    packed.probes = probes.0;
}

pub fn quality_level(quality: RuntimeQuality) -> u32 {
    quality.level()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name of the sky chosen among sky lights with these intensities and names, spawned in
    /// this order.
    fn chosen(skies: &[(f32, &str)]) -> Option<String> {
        let mut app = App::new();
        app.init_resource::<LightChoices>()
            .add_systems(Update, choose_sky);
        for &(intensity, name) in skies {
            let light = DynamicLight {
                intensity,
                ..DynamicLight::sky()
            };
            let visible = InheritedVisibility::VISIBLE;
            let name = Name::new(name.to_string());
            app.world_mut().spawn((light, name, visible));
        }
        app.update();
        let sky = app.world().resource::<LightChoices>().sky?;
        app.world().get::<Name>(sky).map(|name| name.to_string())
    }

    #[test]
    fn one_sky_is_chosen_whatever_the_spawn_order() {
        assert_eq!(chosen(&[(1.0, "a"), (2.0, "b")]).as_deref(), Some("b"));
        assert_eq!(chosen(&[(2.0, "b"), (1.0, "a")]).as_deref(), Some("b"));
        assert_eq!(chosen(&[(1.0, "b"), (1.0, "a")]).as_deref(), Some("a"));
        assert_eq!(chosen(&[(1.0, "a"), (1.0, "b")]).as_deref(), Some("a"));
        assert_eq!(chosen(&[(1.0, "only")]).as_deref(), Some("only"));
        assert_eq!(chosen(&[]), None);
    }
}
