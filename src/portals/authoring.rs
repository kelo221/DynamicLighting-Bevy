//! What a level author places: [`Room`] cells, [`Portal`] openings between them, and
//! [`PortalSettings`].

use bevy::prelude::*;

/// A room cell: the box of half size `half_size` around the entity, under its whole global
/// transform (a parent's shear included). Cells must not overlap, must reach the portals in
/// their walls (at a doorway the cell runs to the wall's middle), and must be closed to the
/// camera everywhere else: every opening it can be seen through needs a [`Portal`].
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default, Debug, Clone, PartialEq)]
#[require(Transform)]
pub struct Room {
    pub half_size: Vec3,
}

impl Room {
    /// A cell `size` meters across.
    pub fn new(size: Vec3) -> Self {
        Self {
            half_size: size * 0.5,
        }
    }
}

impl Default for Room {
    fn default() -> Self {
        Self::new(Vec3::ONE)
    }
}

/// What one side of a [`Portal`] opens into.
#[derive(Reflect, Clone, Debug, Default, PartialEq)]
#[reflect(Default, Debug, Clone, PartialEq)]
pub enum PortalSide {
    /// Found from the cells around the portal: the one cell that takes the whole opening
    /// within [`PortalSettings::tolerance`] of it, or the outside when no cell lies within a
    /// meter of it. Anything else turns portal culling off, with a warning.
    #[default]
    Auto,
    /// These rooms, and the outside when `outside` is set. Listing more than the side really
    /// opens into is always safe; it only culls less.
    Into { rooms: Vec<Entity>, outside: bool },
}

/// An opening between rooms, or between a room and the outside: the rectangle `size` in the
/// entity's local XY plane, open both ways. `front` is the side its local +Z points to.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, Debug, Clone, PartialEq)]
#[require(Transform)]
pub struct Portal {
    pub size: Vec2,
    pub front: PortalSide,
    pub back: PortalSide,
}

impl Portal {
    /// A `size` opening linked automatically on both sides.
    pub fn new(size: Vec2) -> Self {
        Self {
            size,
            front: PortalSide::Auto,
            back: PortalSide::Auto,
        }
    }
}

impl Default for Portal {
    fn default() -> Self {
        Self::new(Vec2::ONE)
    }
}

/// How rooms and portals cull.
#[derive(Resource, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Resource, Default, Debug, Clone, PartialEq)]
pub struct PortalSettings {
    /// Portal culling at all. Off, only the cameras' frusta cull.
    pub enabled: bool,
    /// Portals deep a view is traced; a view that still sees a portal there culls nothing.
    pub depth_limit: u32,
    /// Meters of slack where cells meet and where portals sit on them.
    pub tolerance: f32,
}

impl Default for PortalSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            depth_limit: 16,
            tolerance: 0.02,
        }
    }
}
