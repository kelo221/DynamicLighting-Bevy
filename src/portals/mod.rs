//! Rooms and portals: convex [`Room`] cells joined by [`Portal`] openings. Every frame each
//! camera's view is traced through the portals into its [`RoomView`], and the slices that pick
//! realtime lights, shadow cubemaps and fog volumes ask [`sees`] whether a sphere can reach
//! anything a camera sees.
//!
//! Culling is conservative: a light is dropped only when its whole sphere of influence lies
//! outside every view; walls are never assumed to stop light. Whatever the slice can't be sure
//! of (overlapping rooms, a portal whose sides are unclear, singular transforms, orthographic
//! cameras, views past the trace's limits) culls nothing, so the frusta cull alone.
//!
//! The design follows Godot 3's rooms and portals (lawnjelly, MIT); no code is ported.

mod authoring;
mod cell;
mod graph;
mod link;
mod near;
mod sight;
mod state;
mod systems;
mod trace;

#[cfg(test)]
mod brute_tests;
#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod link_tests;
#[cfg(test)]
mod near_tests;
#[cfg(test)]
mod system_tests;
#[cfg(test)]
mod trace_tests;

use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;
use bevy::transform::TransformSystems;

pub use authoring::{Portal, PortalSettings, PortalSide, Room};
pub use cell::{Aperture, Cell, Parallelepiped};
pub use graph::{Graph, OUTSIDE, PortalNode};
pub use sight::sees;
pub use state::{PortalState, PortalStats, RoomGraph, RoomView};
pub use trace::TracedView;

/// Ordering of the portal work in `PostUpdate`: the graph, then the cameras' views. The slices
/// that pick lights run after [`PortalSystems::Trace`].
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortalSystems {
    /// The [`RoomGraph`] is rebuilt if rooms or portals changed, after transforms propagate.
    Link,
    /// Every camera's [`RoomView`] is traced, after the frusta are updated.
    Trace,
}

pub struct PortalsPlugin;

impl Plugin for PortalsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Room>()
            .register_type::<Portal>()
            .register_type::<PortalSettings>()
            .register_type::<PortalStats>()
            .init_resource::<PortalSettings>()
            .init_resource::<RoomGraph>()
            .init_resource::<PortalStats>()
            .configure_sets(
                PostUpdate,
                (PortalSystems::Link, PortalSystems::Trace).chain(),
            )
            .configure_sets(
                PostUpdate,
                PortalSystems::Link.after(TransformSystems::Propagate),
            )
            .configure_sets(
                PostUpdate,
                PortalSystems::Trace.after(VisibilitySystems::UpdateFrusta),
            )
            .add_systems(
                PostUpdate,
                (
                    systems::build_graph.in_set(PortalSystems::Link),
                    systems::trace_views.in_set(PortalSystems::Trace),
                ),
            );
    }
}
