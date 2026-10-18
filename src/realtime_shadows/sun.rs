//! The sun's realtime shadows: the brightest sun with `ShadowMode::Realtime` gets cascades over
//! the view of the camera the realtime lights are chosen for. A cascade renders again only when
//! it snaps to a new place, a caster it holds changes, or its last rendering missed a caster. A
//! sun standing where it was baked draws only meshes without bake data, like the cubemaps of
//! lights in place: the bake has the shadows of the rest.

use bevy::camera::visibility::{SetViewVisibility, ViewVisibility};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::renderer::RenderDevice;
use bevy::render::sync_world::SyncToRenderWorld;

use super::cascades::{CascadePose, CascadeViewer, fit_cascades, plan_cascades};
use super::casters::Caster;
use super::cubemaps::{SUN_CASCADES, cascade_array, shadow_cubemap_format};
use super::faces::select_shadowed_sun;
use super::tracking::{CasterBounds, CasterChanges};
use super::views::{CasterQuery, ShadowCubemapStats, caster_of};
use crate::light::{BakedInPlace, DynamicLight, LightAttachments, LightType, ShadowMode};
use crate::shared::{
    DynamicGeometryMode, DynamicLightingSettings, GpuSunShadow, NamePaths, RuntimeQuality,
    SUN_CASCADE_COUNT, SunShadowBlock,
};

/// One cascade of the sun's realtime shadows: layer `index` of the cascade array.
#[derive(Component, Clone, Debug, PartialEq)]
#[require(SyncToRenderWorld)]
pub struct SunCascadeView {
    pub index: u32,
    /// Where it renders from; `None` while no sun has realtime shadows.
    pub pose: Option<CascadePose>,
    /// The cascade renders this frame.
    pub renders: bool,
    /// What it draws when it renders.
    pub casters: Vec<Entity>,
}

/// The cascade views, their last renderings and this frame's poses.
#[derive(Resource, Default, Debug)]
pub(super) struct SunCascades {
    views: Vec<Entity>,
    rendered: [Option<CascadePose>; SUN_CASCADE_COUNT],
    poses: Option<[CascadePose; SUN_CASCADE_COUNT]>,
    /// The cascades rendering this frame, one bit each.
    rendering: u8,
    /// The format of the full-size cascade array, once made.
    allocated: Option<TextureFormat>,
    /// More than one sun asked for realtime shadows, and a warning said so.
    warned: bool,
}

type SunItem<'a> = (
    Entity,
    &'a DynamicLight,
    &'a GlobalTransform,
    &'a InheritedVisibility,
    Has<BakedInPlace>,
);

/// The sun with realtime shadows, if any: suns asking for them, visible and valid.
fn shadowed_sun(suns: &Query<SunItem>, paths: &NamePaths, warned: &mut bool) -> Option<Entity> {
    let candidates: Vec<(Entity, f32, Vec<String>)> = suns
        .iter()
        .filter(|(_, light, _, visible, _)| {
            light.light_type == LightType::Sun
                && light.shadows == ShadowMode::Realtime
                && light.is_valid()
                && visible.get()
        })
        .map(|(entity, light, ..)| (entity, light.intensity, paths.name_path(entity)))
        .collect();
    let sun = select_shadowed_sun(&candidates)?;
    if candidates.len() > 1 && !*warned {
        *warned = true;
        let path = candidates
            .iter()
            .find(|(entity, ..)| *entity == sun)
            .map(|(.., path)| path.join("/"))
            .unwrap_or_default();
        warn!(
            "dynlight shadows: {} suns have realtime shadows; only one is supported, \"{path}\"",
            candidates.len()
        );
    }
    Some(sun)
}

/// Fits the cascades of the shadowed sun to the camera, decides which render again, and hands
/// the lit shader their [`SunShadowBlock`] and the sun its flag in [`LightAttachments`].
#[allow(clippy::too_many_arguments)]
pub(super) fn place_sun_cascades(
    mut commands: Commands,
    settings: Res<DynamicLightingSettings>,
    suns: Query<SunItem>,
    viewers: Query<(&Camera, &GlobalTransform)>,
    paths: NamePaths,
    (known, changes): (Res<CasterBounds>, Res<CasterChanges>),
    mut cascades: ResMut<SunCascades>,
    mut views: Query<&mut SunCascadeView>,
    mut block: ResMut<SunShadowBlock>,
    mut attachments: ResMut<LightAttachments>,
    mut images: ResMut<Assets<Image>>,
    mut stats: ResMut<ShadowCubemapStats>,
    device: Option<Res<RenderDevice>>,
) {
    let cascades = &mut *cascades;
    let sun = shadowed_sun(&suns, &paths, &mut cascades.warned);
    let viewer = viewers
        .iter()
        .filter(|(camera, _)| camera.is_active)
        .max_by_key(|(camera, _)| camera.order)
        .map(|(camera, transform)| CascadeViewer {
            world_from_view: transform.to_matrix(),
            clip_from_view: camera.clip_from_view(),
        });
    let enabled = settings.quality != RuntimeQuality::IntegratedGraphics;
    let shadowed = sun
        .zip(viewer)
        .zip(device)
        .filter(|_| enabled)
        .and_then(|((sun, viewer), device)| Some((suns.get(sun).ok()?, viewer, device)));
    let Some(((sun, _, transform, _, in_place), viewer, device)) = shadowed else {
        block.0 = GpuSunShadow::default();
        attachments.sun_cascades = None;
        cascades.rendered = [None; SUN_CASCADE_COUNT];
        cascades.poses = None;
        cascades.rendering = 0;
        stats.cascades_rendered = 0;
        for &view in &cascades.views {
            if let Ok(mut view) = views.get_mut(view) {
                view.renders = false;
            }
        }
        return;
    };

    // As for cubemaps, moving meshes that ignore the bake need every caster.
    let moving_only = in_place && settings.dynamic_geometry != DynamicGeometryMode::LightingOnly;
    let spheres = known.spheres(moving_only);
    let to_sun = transform.back().as_vec3();
    let poses = fit_cascades(
        &viewer,
        to_sun,
        settings.sun_shadow_distance,
        moving_only,
        &spheres,
    );
    let format = shadow_cubemap_format(device.features());
    let recreated = cascades.allocated != Some(format)
        && images
            .insert(&SUN_CASCADES, cascade_array(true, format))
            .is_ok();
    if recreated {
        cascades.allocated = Some(format);
    }
    let reset = recreated || changes.settle > 0;
    let rendering = plan_cascades(
        &mut cascades.rendered,
        &poses,
        &changes.changes,
        changes.redraw_cascades,
        reset,
    );
    cascades.poses = Some(poses);
    cascades.rendering = rendering;
    block.0 = GpuSunShadow {
        cascades: poses.map(|pose| pose.gpu()),
        count: SUN_CASCADE_COUNT as u32,
        flags: if moving_only {
            GpuSunShadow::FLAG_MOVING_ONLY
        } else {
            0
        },
        _padding: [0; 2],
    };
    attachments.sun_cascades = Some(sun);
    stats.cascades_rendered = rendering.count_ones();

    for (index, pose) in poses.into_iter().enumerate() {
        let renders = rendering & (1 << index) != 0;
        match cascades.views.get(index) {
            Some(&entity) => {
                if let Ok(mut view) = views.get_mut(entity) {
                    view.pose = Some(pose);
                    view.renders = renders;
                }
            }
            None => {
                let view = SunCascadeView {
                    index: index as u32,
                    pose: Some(pose),
                    renders,
                    casters: Vec::new(),
                };
                let name = Name::new("dynlight sun cascade");
                cascades.views.push(commands.spawn((name, view)).id());
            }
        }
    }
}

/// Lists the casters of every cascade rendering this frame and marks them visible, so Bevy
/// extracts them even when no camera sees them.
pub(super) fn cull_sun_casters(
    cascades: Res<SunCascades>,
    mut views: Query<&mut SunCascadeView>,
    casters: Query<CasterQuery, With<Mesh3d>>,
    mut visibility: Query<&mut ViewVisibility>,
    mut found: Local<Vec<(Entity, Caster)>>,
) {
    let Some(poses) = cascades.poses.filter(|_| cascades.rendering != 0) else {
        return;
    };
    found.clear();
    found.extend(casters.iter().map(caster_of));
    for &entity in &cascades.views {
        let Ok(mut view) = views.get_mut(entity) else {
            continue;
        };
        view.casters.clear();
        if !view.renders {
            continue;
        }
        let pose = poses[view.index as usize];
        for &(caster, bounds) in found.iter() {
            if pose.draws(bounds) {
                view.casters.push(caster);
                if let Ok(mut visible) = visibility.get_mut(caster) {
                    visible.set_visible();
                }
            }
        }
    }
}
