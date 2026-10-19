//! Opt-in low-overhead scheduling: the app's per-frame schedules on Bevy's single-threaded
//! executor.
//!
//! Bevy's multi-threaded executor hands systems to worker threads and waits for them to come
//! back. A frame runs several hundred systems, most of them for a few microseconds, so in a
//! scene like dynlight's stress hall those round trips cost more than the systems themselves,
//! about 1 ms a frame with dynlight and with Bevy's own lights alike (see the README's
//! benchmarks). [`SingleThreadedSchedulesPlugin`] runs those schedules on the thread that runs
//! them instead.
//!
//! What stays the same: systems that split their own queries across threads (`par_iter`, such as
//! transform propagation and visibility) still do, the render world still runs beside the main
//! world, and commands are applied at the same points. One exception: an exclusive system with
//! commands of its own, such as an exclusive system piped into one with `Commands`, has them
//! applied at the next sync point rather than when it returns.
//!
//! What changes: systems that may run in either order now run in one order, fixed when the
//! schedule is built, and each switched schedule runs all its systems on the thread that runs
//! it. For the render world that is the render thread with pipelined rendering, except for
//! extraction, which runs on the main thread. A render system that needs the main thread (for
//! `NonSend` data created there) loses it; Bevy's `create_surfaces` on macOS and iOS is why
//! `Render` keeps its executor there.
//!
//! Executors are an app-wide choice, so
//! [`DynamicLightingPlugin`](crate::DynamicLightingPlugin) doesn't add this plugin. An app with
//! long independent systems can be faster with the multi-threaded executor; measure both.

mod executors;
#[cfg(test)]
mod tests;

use bevy::prelude::*;
use bevy::render::RenderApp;

/// Runs Bevy's per-frame schedules on Bevy's single-threaded executor. Add it with the other
/// plugins; it switches the schedules once, in its `finish`, after every plugin's `build`.
///
/// - `main`: Bevy's main schedules from `First` to `Last` (with `StateTransition` and
///   `SpawnScene`) and its fixed-timestep schedules from `FixedFirst` to `FixedLast`. Startup
///   schedules keep their executor.
/// - `render`: the render world's extraction, `First`, `Render`, the schedule that runs them,
///   the render graph and the `Core3d` and `Core2d` camera schedules. On macOS and iOS `Render`
///   keeps its executor, because window surfaces have to be created on the main thread there.
///
/// Schedules an app adds keep their executor and their settings, and so do schedules created
/// after this plugin's `finish` (in another plugin's `finish` or `cleanup`, or at runtime).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SingleThreadedSchedulesPlugin {
    pub main: bool,
    pub render: bool,
}

impl Default for SingleThreadedSchedulesPlugin {
    fn default() -> Self {
        Self {
            main: true,
            render: true,
        }
    }
}

impl Plugin for SingleThreadedSchedulesPlugin {
    fn build(&self, _app: &mut App) {}

    /// After every plugin's `build`, and before pipelined rendering moves the render app to its
    /// own thread in `cleanup`.
    fn finish(&self, app: &mut App) {
        let main = if self.main {
            executors::switch_main(app.main_mut())
        } else {
            0
        };
        let render = match app.get_sub_app_mut(RenderApp) {
            Some(render_app) if self.render => executors::switch_render(render_app),
            _ => 0,
        };
        debug!("dynlight scheduling: {main} main and {render} render schedules single-threaded");
    }
}
