//! Builds the sky's lighting data off the main thread when a sky's image loads or changes, and
//! tells the background pass which sky to draw.

use bevy::asset::AssetEvent;
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on};

use super::equirect::Pyramid;
use super::frame::first_sky;
use super::images::{
    SkyImageError, background_image, equirect_from_image, placeholder_background,
    placeholder_specular, specular_image,
};
use super::prefilter::{SPECULAR_LEVELS, SPECULAR_RESOLUTION, SPECULAR_SAMPLES, prefilter};
use super::sh::{irradiance_over_pi, project, to_gpu};
use super::{SKY_BACKGROUND, SKY_SPECULAR, SkyBackground, SkyEnvironment, SkyStatus};
use crate::light::{DynamicLight, LightType, PackedLights};
use crate::shared::{GpuSkyIrradiance, SkyIrradiance};

/// The widest pyramid level the diffuse light is projected from; three bands need no more.
const SH_SOURCE_WIDTH: u32 = 512;

/// What a sky's lighting data is built from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SkySource {
    image: AssetId<Image>,
    max_radiance: Option<f32>,
}

/// The sky's lighting data, ready to upload.
pub(super) struct SkyData {
    irradiance: GpuSkyIrradiance,
    specular: Image,
    background: Image,
    texels: usize,
    milliseconds: u128,
}

#[cfg(test)]
impl SkyData {
    /// The irradiance and the background image's bytes.
    pub(super) fn parts(&self) -> (&GpuSkyIrradiance, &[u8]) {
        let background = self.background.data.as_deref().unwrap_or_default();
        (&self.irradiance, background)
    }
}

type SkyTask = Task<Result<SkyData, SkyImageError>>;

#[derive(Resource, Default)]
pub(super) struct SkyBuilds {
    /// The source the current images and irradiance came from.
    built: Option<SkySource>,
    running: Option<(SkySource, SkyTask)>,
    /// The running build's image changed after it started.
    restart: bool,
    warned_extra_skies: bool,
}

/// The sky's diffuse light, specular cube and background from its image.
pub(super) fn build_sky(
    source: &Image,
    max_radiance: Option<f32>,
) -> Result<SkyData, SkyImageError> {
    let start = Instant::now();
    let base = equirect_from_image(source)?;
    let texels = base.texels.len();
    let background = Pyramid::new(base);
    let clamped = max_radiance.map(|max| Pyramid::new(background.base().clamped(max)));
    let lighting = clamped.as_ref().unwrap_or(&background);
    let sh_level = lighting
        .levels
        .iter()
        .find(|level| level.width <= SH_SOURCE_WIDTH)
        .unwrap_or(lighting.base());
    let cube = prefilter(
        lighting,
        SPECULAR_RESOLUTION,
        SPECULAR_LEVELS,
        SPECULAR_SAMPLES,
    );
    Ok(SkyData {
        irradiance: to_gpu(&irradiance_over_pi(project(sh_level))),
        specular: specular_image(&cube),
        background: background_image(&background),
        texels,
        milliseconds: start.elapsed().as_millis(),
    })
}

/// Starts a build when the sky's source changes and applies finished builds. One sky lights the
/// scene; with several, the first found is used.
pub(super) fn build_skies(
    skies: Query<(&SkyEnvironment, &DynamicLight)>,
    mut events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
    mut builds: ResMut<SkyBuilds>,
    mut irradiance: ResMut<SkyIrradiance>,
    mut status: ResMut<SkyStatus>,
) {
    let changed: Vec<AssetId<Image>> = events
        .read()
        .filter_map(|event| match event {
            // A first load needs no notice: nothing was built from the image yet.
            AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    let builds = &mut *builds;
    if let Some((source, _)) = &builds.running
        && changed.contains(&source.image)
    {
        builds.restart = true;
    }
    if builds
        .running
        .as_ref()
        .is_some_and(|(_, task)| task.is_finished())
        && let Some((source, task)) = builds.running.take()
    {
        // Finished, so this returns at once.
        match block_on(task) {
            Ok(data) => {
                info!(
                    "dynlight sky: texels {}, ms {}",
                    data.texels, data.milliseconds
                );
                let _ = images.insert(&SKY_SPECULAR, data.specular);
                let _ = images.insert(&SKY_BACKGROUND, data.background);
                irradiance.0 = data.irradiance;
            }
            Err(error) => warn!("dynlight sky: can't light with the sky image: {error}"),
        }
        builds.built = (!core::mem::take(&mut builds.restart)).then_some(source);
    }

    let mut sources = skies
        .iter()
        .filter(|(_, light)| light.light_type == LightType::Sky)
        .map(|(sky, _)| SkySource {
            image: sky.image.id(),
            max_radiance: sky.max_radiance,
        });
    let Some(source) = sources.next() else {
        status.building = false;
        return;
    };
    if !builds.warned_extra_skies && sources.any(|other| other != source) {
        warn!("dynlight sky: more than one sky image; one of them lights the scene");
        builds.warned_extra_skies = true;
    }
    if changed.contains(&source.image) && builds.running.is_none() {
        builds.built = None;
    }
    status.building = builds.built != Some(source);
    if builds.built == Some(source) || builds.running.is_some() {
        return;
    }
    let Some(image) = images.get(source.image) else {
        return;
    };
    let image = image.clone();
    let task =
        AsyncComputeTaskPool::get().spawn(async move { build_sky(&image, source.max_radiance) });
    builds.running = Some((source, task));
}

/// The sky the background pass draws this frame.
pub(super) fn note_sky_background(
    packed: Res<PackedLights>,
    mut background: ResMut<SkyBackground>,
) {
    let sky = first_sky(&packed.globals, &packed.lights);
    if background.0 != sky {
        background.0 = sky;
    }
}

/// Adds the black placeholders that are bound until a sky is built.
pub(super) fn insert_placeholders(app: &mut App) {
    let Some(mut images) = app.world_mut().get_resource_mut::<Assets<Image>>() else {
        return;
    };
    images
        .insert(&SKY_SPECULAR, placeholder_specular())
        .expect("the sky specular handle is a fixed UUID");
    images
        .insert(&SKY_BACKGROUND, placeholder_background())
        .expect("the sky background handle is a fixed UUID");
}
