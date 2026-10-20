//! Reflection probes: boxes whose surroundings are captured into a cube at runtime. Metallic
//! surfaces inside a box reflect its cube with parallax correction (box projection), and meshes
//! without bake data inside it are lit by its irradiance instead of the ambient color.
//!
//! A probe is captured once the scene is ready (no render pipeline compiling, the bake applied,
//! the sky built, the lit shader's features chosen), again when the probe changes, the bake or
//! the sky changes, and on [`CaptureReflectionProbes`]. Six 90° cameras render the faces, which
//! are read back and prefiltered on a background task by the environment slice's code, like the
//! sky. The cubes go into [`REFLECTION_CUBES`], which every lit material binds: the sky's cube
//! is cube 0 and the probes follow.
//!
//! Shader module: `dynlight::reflection_probes::shaders::probes`.

pub mod assemble;
mod capture;
pub mod records;
#[cfg(test)]
pub(crate) mod shader_tests;
mod systems;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::asset::{AssetEventSystems, uuid_handle};
use bevy::prelude::*;
use bevy::render::render_resource::PipelineCache;
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::shader::load_shader_library;
use bevy::transform::TransformSystems;

use crate::light::LightSystems;
use crate::shared::{ChooseFeatures, shaders_available};

/// The cube array every lit material reflects (unless it has a cube of its own): the sky's
/// prefiltered cube, then one per captured probe.
pub const REFLECTION_CUBES: Handle<Image> = uuid_handle!("b4e19c27-6d3f-4a85-9c1e-2f7a8d0b5e63");

/// A reflection probe. Its box is the entity's local ±1 cube: centred at the translation,
/// turned by the rotation, with the scale as half extents, as for a [`Room`](crate::portals::Room).
/// Make it fit the room it stands in: box projection treats the box's sides as the walls the
/// cube saw. Where boxes overlap, the smallest one holding a point is used; outside every box
/// surfaces reflect the sky.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default, Debug, Clone, PartialEq)]
#[require(Transform, Visibility)]
pub struct ReflectionProbe {
    /// Texels per side of the captured faces. The prefiltered cube is always 128² at its
    /// sharpest level.
    pub resolution: u32,
    /// Scales the irradiance meshes without bake data get from the probe. The capture sees
    /// emissive surfaces, so turn it down when lights already stand in for them.
    pub diffuse_intensity: f32,
    /// Where the cube is captured, from the box's center, in world units. Keep it inside the
    /// box, away from furniture: the reflection is exact for walls seen from there.
    pub capture_offset: Vec3,
}

impl Default for ReflectionProbe {
    fn default() -> Self {
        Self {
            resolution: 128,
            diffuse_intensity: 1.0,
            capture_offset: Vec3::ZERO,
        }
    }
}

/// Captures every reflection probe again, for example after lights changed.
#[derive(Message, Clone, Copy, Debug, Default)]
pub struct CaptureReflectionProbes;

/// Where the probes stand.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReflectionProbeStatus {
    /// Probes waiting for a capture or being captured.
    pub pending: usize,
    /// Probes whose cube the lit materials reflect.
    pub captured: usize,
}

/// Render pipelines still compiling, written by the render world and read by the main world.
#[derive(Resource, Clone)]
pub struct WaitingPipelines(Arc<AtomicUsize>);

impl WaitingPipelines {
    /// Whether any render pipeline is compiling, or the render world hasn't said yet.
    pub fn compiling(&self) -> bool {
        self.0.load(Ordering::Relaxed) != 0
    }

    /// Adds the resource to `app` and its render app, once.
    pub fn track(app: &mut App) {
        if app.world().contains_resource::<Self>() {
            return;
        }
        let waiting = Self(Arc::new(AtomicUsize::new(usize::MAX)));
        app.insert_resource(waiting.clone());
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(waiting).add_systems(
                Render,
                count_waiting_pipelines.in_set(RenderSystems::Cleanup),
            );
        }
    }
}

fn count_waiting_pipelines(cache: Res<PipelineCache>, waiting: Res<WaitingPipelines>) {
    waiting
        .0
        .store(cache.waiting_pipelines().count(), Ordering::Relaxed);
}

pub struct ReflectionProbesPlugin;

impl Plugin for ReflectionProbesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<ReflectionProbe>()
            .add_message::<CaptureReflectionProbes>()
            .init_resource::<ReflectionProbeStatus>();
    }

    fn finish(&self, app: &mut App) {
        if !app.world().contains_resource::<Assets<Image>>() {
            return;
        }
        WaitingPipelines::track(app);
        systems::insert_cube_array(app);
        app.init_resource::<systems::Probes>()
            .add_systems(
                PostUpdate,
                (
                    systems::mark_stale.after(TransformSystems::Propagate),
                    systems::update_probes
                        .after(AssetEventSystems)
                        .before(LightSystems::Pack),
                )
                    .chain(),
            )
            .add_systems(Last, capture::run_captures.before(ChooseFeatures));
        if shaders_available(app) {
            load_shader_library!(app, "shaders/probes.wesl");
        }
    }
}
