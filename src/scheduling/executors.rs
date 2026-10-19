//! Which schedules switch to the single-threaded executor, and switching them.

use bevy::app::{FixedMainScheduleOrder, MainScheduleOrder, SubApp};
use bevy::core_pipeline::{Core2d, Core3d};
use bevy::ecs::schedule::{InternedScheduleLabel, ScheduleLabel, SingleThreadedExecutor};
use bevy::prelude::*;
use bevy::render::renderer::RenderGraph;
use bevy::render::{ExtractSchedule, RenderScheduleOrder};

/// Puts `schedule` on the single-threaded executor. The new executor brings its own settings,
/// so whether the schedule applies its systems' commands when it ends is set again here.
pub(super) fn single_threaded(schedule: &mut Schedule, apply_final_deferred: bool) {
    schedule
        .set_executor(SingleThreadedExecutor::new())
        .set_apply_final_deferred(apply_final_deferred);
}

/// The main app's per-frame schedules, in the order it runs them.
fn main_schedules(world: &World) -> Vec<InternedScheduleLabel> {
    let main = world.get_resource::<MainScheduleOrder>();
    let fixed = world.get_resource::<FixedMainScheduleOrder>();
    main.into_iter()
        .flat_map(|order| order.labels.iter())
        .chain(fixed.into_iter().flat_map(|order| order.labels.iter()))
        .copied()
        .collect()
}

/// The render world's schedules that apply their commands when they end: the render schedule
/// order, the render app's update schedule that runs it, the render graph and the camera
/// schedules.
fn render_schedules(render_app: &SubApp) -> Vec<InternedScheduleLabel> {
    let mut labels: Vec<InternedScheduleLabel> = render_app
        .world()
        .get_resource::<RenderScheduleOrder>()
        .map(|order| order.labels.clone())
        .unwrap_or_default();
    labels.extend(render_app.update_schedule);
    labels.extend([RenderGraph.intern(), Core3d.intern(), Core2d.intern()]);
    // `create_surfaces` must run on the main thread there, which only the multi-threaded
    // executor arranges.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    labels.retain(|&label| label != bevy::render::Render.intern());
    labels
}

/// Switches each of `labels` that `app` has, once; schedules it doesn't have aren't created.
/// Returns how many it switched.
fn switch(app: &mut SubApp, labels: &[InternedScheduleLabel], apply_final_deferred: bool) -> usize {
    let mut switched = Vec::new();
    for &label in labels {
        if switched.contains(&label) {
            continue;
        }
        if let Some(schedule) = app.get_schedule_mut(label) {
            single_threaded(schedule, apply_final_deferred);
            switched.push(label);
        }
    }
    switched.len()
}

pub(super) fn switch_main(app: &mut SubApp) -> usize {
    let labels = main_schedules(app.world());
    switch(app, &labels, true)
}

pub(super) fn switch_render(render_app: &mut SubApp) -> usize {
    let labels = render_schedules(render_app);
    // Bevy builds extraction without applying its commands at the end: the render world applies
    // them itself, on the render thread, while the main world runs the next frame.
    switch(render_app, &labels, true) + switch(render_app, &[ExtractSchedule.intern()], false)
}
