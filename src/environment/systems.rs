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
    placeholder_specular, specular_image, uniform_specular,
};
use super::prefilter::{SPECULAR_LEVELS, SPECULAR_RESOLUTION, SPECULAR_SAMPLES, prefilter};
use super::sh::{irradiance_over_pi, project, to_gpu, uniform};
use super::{SKY_BACKGROUND, SKY_SPECULAR, SkyBackground, SkyEnvironment, SkyStatus};
use crate::light::{LightChoices, PackedLights};
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

/// What the bound sky data shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Shown {
    /// Black: nothing yet, or an image sky on its way.
    #[default]
    Nothing,
    /// The uniform white sky of sky lights without an image.
    Uniform,
    /// A built image.
    Image,
}

#[derive(Resource, Default)]
pub(super) struct SkyBuilds {
    shown: Shown,
    /// The source the current images and irradiance came from.
    built: Option<SkySource>,
    /// A source whose build failed; it isn't tried again until its image changes.
    failed: Option<SkySource>,
    running: Option<(SkySource, SkyTask)>,
    /// The running build's image changed after it started.
    restart: bool,
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
/// scene, the one in [`LightChoices`].
pub(super) fn build_skies(
    chosen: Res<LightChoices>,
    skies: Query<&SkyEnvironment>,
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
        let current = (!core::mem::take(&mut builds.restart)).then_some(source);
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
                builds.shown = Shown::Image;
                builds.built = current;
            }
            Err(error) => {
                warn!("dynlight sky: can't light with the sky image: {error}");
                // An earlier sky's data would light the scene in its place.
                let _ = images.insert(&SKY_SPECULAR, placeholder_specular());
                irradiance.0 = GpuSkyIrradiance::default();
                builds.shown = Shown::Nothing;
                (builds.built, builds.failed) = (None, current);
            }
        }
    }

    let source = chosen
        .sky
        .and_then(|sky| skies.get(sky).ok())
        .map(|sky| SkySource {
            image: sky.image.id(),
            max_radiance: sky.max_radiance,
        });
    let Some(source) = source else {
        *status = SkyStatus::default();
        // Sky lights without an image light like a uniform white sky.
        (builds.running, builds.restart) = (None, false);
        (builds.built, builds.failed) = (None, None);
        if builds.shown != Shown::Uniform {
            let _ = images.insert(&SKY_SPECULAR, uniform_specular());
            irradiance.0 = to_gpu(&uniform(Vec3::ONE));
            builds.shown = Shown::Uniform;
        }
        return;
    };
    if builds.shown == Shown::Uniform {
        // An image sky adds no light until it is built.
        let _ = images.insert(&SKY_SPECULAR, placeholder_specular());
        irradiance.0 = GpuSkyIrradiance::default();
        builds.shown = Shown::Nothing;
    }
    if changed.contains(&source.image) {
        builds.failed = None;
        if builds.running.is_none() {
            builds.built = None;
        }
    }
    let failed = builds.failed == Some(source);
    *status = SkyStatus {
        building: builds.built != Some(source) && !failed,
        failed,
    };
    if builds.built == Some(source) || failed || builds.running.is_some() {
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

/// The sky the background pass draws this frame. A sky without an image leaves the cameras'
/// clear color.
pub(super) fn note_sky_background(
    packed: Res<PackedLights>,
    builds: Option<Res<SkyBuilds>>,
    mut background: ResMut<SkyBackground>,
) {
    let sky = if builds.is_some_and(|builds| builds.shown == Shown::Image) {
        first_sky(&packed.globals, &packed.lights)
    } else {
        None
    };
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
