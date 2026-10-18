//! Renders every shadow view in one go. A single render-world entity, which Bevy's camera driver
//! runs before the cameras like any `RootNonCameraView`, runs a schedule that preprocesses the
//! meshes of every face and cascade view in one compute pass, then draws them all into one
//! command encoder (`render::shadow_face_pass`). Each view running Bevy's whole `Core3d` schedule
//! instead cost a schedule run, four command encoders and two empty compute passes per view.
//!
//! The views draw directly (`NoIndirectDrawing`), so the only part of Bevy's mesh preprocessing
//! they need is the direct dispatch of `early_gpu_preprocess`. This repeats it for the shadow
//! phase, with Bevy's own pipeline, bind groups and work items.

use core::any::TypeId;

use bevy::core_pipeline::schedule::RootNonCameraView;
use bevy::ecs::entity::EntityHashSet;
use bevy::ecs::schedule::{ScheduleBuildSettings, ScheduleLabel, SingleThreadedExecutor};
use bevy::pbr::{
    MeshInputUniform, MeshUniform, PhasePreprocessBindGroups, PreprocessBindGroups,
    PreprocessPipelines,
};
use bevy::prelude::*;
use bevy::render::batching::gpu_preprocessing::{
    BatchedInstanceBuffers, PreprocessWorkItemBuffers,
};
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::render_resource::{BindGroup, ComputePassDescriptor, PipelineCache};
use bevy::render::renderer::RenderContext;
use bevy::render::view::{ExtractedView, ViewUniformOffset};

use super::extract::ShadowFaceView;
use super::phase::ShadowFace3d;

/// The workgroup size of Bevy's mesh preprocessing shader (`WORKGROUP_SIZE` in `bevy_pbr` 0.20,
/// private there).
const PREPROCESS_WORKGROUP_SIZE: usize = 64;

/// The schedule the shadow driver entity runs.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ShadowViews;

/// One thread, its systems in order, and their command buffers applied at the end of the run,
/// so the preprocessing is submitted before the draws.
pub(super) fn shadow_views_schedule() -> Schedule {
    let mut schedule = Schedule::new(ShadowViews);
    schedule.set_build_settings(ScheduleBuildSettings {
        auto_insert_apply_deferred: false,
        ..default()
    });
    // `SingleThreadedExecutor::default()` would not apply the buffers at the end.
    schedule.set_executor(SingleThreadedExecutor::new());
    schedule.set_apply_final_deferred(true);
    schedule
}

pub(super) fn spawn_shadow_driver(mut commands: Commands) {
    commands.spawn((
        Name::new("dynlight shadow views"),
        RootNonCameraView(ShadowViews.intern()),
    ));
}

/// The shadow views whose meshes are ready to draw this frame. A view whose preprocessing was
/// skipped would draw last frame's mesh transforms, so it renders again later instead.
#[derive(Resource, Default)]
pub(super) struct PreprocessedShadowViews(pub EntityHashSet);

type PreprocessView<'a> = (
    Entity,
    &'a ExtractedView,
    Option<&'a PreprocessBindGroups>,
    Option<&'a ViewUniformOffset>,
);

/// Bevy's direct mesh preprocessing for every shadow view, in one compute pass.
pub(super) fn preprocess_shadow_views(
    views: Query<PreprocessView, With<ShadowFaceView>>,
    buffers: Option<Res<BatchedInstanceBuffers<MeshUniform, MeshInputUniform>>>,
    pipelines: Option<Res<PreprocessPipelines>>,
    cache: Res<PipelineCache>,
    mut preprocessed: ResMut<PreprocessedShadowViews>,
    mut ctx: RenderContext,
) {
    preprocessed.0.clear();
    // Without Bevy's GPU instance buffer builder the meshes are prepared on the CPU.
    let Some(buffers) = buffers else {
        preprocessed
            .0
            .extend(views.iter().map(|(entity, ..)| entity));
        return;
    };
    let phase = TypeId::of::<ShadowFace3d>();
    let pipeline = pipelines
        .and_then(|pipelines| pipelines.direct_preprocess.pipeline_id)
        .and_then(|id| cache.get_compute_pipeline(id));
    let (Some(work_items), Some(pipeline)) = (buffers.phase_instance_buffers.get(&phase), pipeline)
    else {
        return;
    };
    let mut dispatches: Vec<(&BindGroup, u32, u32)> = Vec::new();
    for (entity, view, groups, offset) in &views {
        let work = work_items.work_item_buffers.get(&view.retained_view_entity);
        let group = groups.and_then(|groups| groups.0.get(&phase));
        let (
            Some(PreprocessWorkItemBuffers::Direct(work)),
            Some(PhasePreprocessBindGroups::Direct(group)),
            Some(offset),
        ) = (work, group, offset)
        else {
            continue;
        };
        let workgroups = work.len().div_ceil(PREPROCESS_WORKGROUP_SIZE) as u32;
        if workgroups > 0 {
            dispatches.push((group, offset.offset, workgroups));
        }
        preprocessed.0.insert(entity);
    }
    if dispatches.is_empty() {
        return;
    }
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("dynlight_shadow_preprocess"),
            timestamp_writes: None,
        });
    let span = diagnostics.pass_span(&mut pass, "dynlight_shadow_preprocess");
    pass.set_pipeline(pipeline);
    for (group, offset, workgroups) in dispatches {
        pass.set_bind_group(0, group, &[offset]);
        pass.dispatch_workgroups(workgroups, 1, 1);
    }
    span.end(&mut pass);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource)]
    struct Applied;

    #[test]
    fn the_schedule_applies_its_systems_buffers_when_it_ends() {
        let mut world = World::new();
        let mut schedule = shadow_views_schedule();
        schedule.add_systems(|mut commands: Commands| commands.insert_resource(Applied));
        schedule.run(&mut world);
        assert!(
            world.contains_resource::<Applied>(),
            "command buffers would never be submitted"
        );
    }
}
