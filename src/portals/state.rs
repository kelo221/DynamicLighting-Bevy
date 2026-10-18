//! What the portal systems compute: the [`RoomGraph`], each camera's [`RoomView`] and the
//! [`PortalStats`].

use bevy::prelude::*;

use super::graph::Graph;
use super::trace::TracedView;

/// Whether portal culling is working.
#[derive(Reflect, Clone, Debug, Default, PartialEq, Eq)]
pub enum PortalState {
    /// No rooms (or culling is off): the cameras' frusta cull alone.
    #[default]
    Empty,
    /// The graph is valid and the cameras' views are traced through it.
    Ready,
    /// The rooms or portals break the authoring rules, so nothing is culled by them. The
    /// reason is also logged.
    Disabled(String),
}

/// The rooms and portals, validated and linked; rebuilt when any of them changes.
#[derive(Resource, Debug, Default)]
pub struct RoomGraph {
    pub(super) generation: u64,
    pub(super) graph: Graph,
    pub(super) state: PortalState,
    pub(super) culprits: Vec<Entity>,
    /// The room behind each cell: node `n` is room `n - 1`.
    pub rooms: Vec<Entity>,
    /// The portal behind each graph portal.
    pub portals: Vec<Entity>,
}

impl RoomGraph {
    pub fn state(&self) -> &PortalState {
        &self.state
    }

    /// The graph, when it is valid.
    pub fn ready(&self) -> Option<&Graph> {
        (self.state == PortalState::Ready).then_some(&self.graph)
    }

    /// The rooms and portals that left the graph [`PortalState::Disabled`].
    pub fn culprits(&self) -> &[Entity] {
        &self.culprits
    }

    /// Bumped on every rebuild; a [`RoomView`] traced with another one is ignored.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// What a camera sees through the portals, traced every frame. The portal systems add it to
/// every camera. Unbounded (orthographic, inactive, no valid graph, past the trace's limits),
/// it culls nothing.
#[derive(Component, Clone, Debug, Default)]
pub struct RoomView {
    pub(super) generation: u64,
    pub(super) bounded: bool,
    pub(super) traced: TracedView,
}

impl RoomView {
    /// The traced view, if it bounds anything for a graph of `generation`.
    pub fn bounded(&self, generation: u64) -> Option<&TracedView> {
        (self.bounded && self.generation == generation).then_some(&self.traced)
    }
}

/// What portal culling did this frame.
#[derive(Resource, Reflect, Clone, Debug, Default, PartialEq)]
#[reflect(Resource, Default, Debug, Clone, PartialEq)]
pub struct PortalStats {
    pub state: PortalState,
    pub rooms: u32,
    pub portals: u32,
    /// Portals with the same nodes on both sides, which join nothing.
    pub ignored_portals: u32,
    /// Cameras traced, and how many of those views were unbounded.
    pub views: u32,
    pub unbounded_views: u32,
    pub visits: u32,
    pub portal_tests: u32,
    pub plane_tests: u32,
}
