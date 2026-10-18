//! The room graph: validated room cells as nodes `1..`, everything outside them as node
//! [`OUTSIDE`], and portals as the edges between them. Built from plain inputs; the systems
//! rebuild it when rooms or portals change.

use bevy::math::{Affine3A, Vec2, Vec3};

use super::cell::{Aperture, Cell, Parallelepiped};
use super::link::{Unresolved, resolve};

/// The node for all the space no room cell holds.
pub const OUTSIDE: usize = 0;

/// Points closer than this to a cell count as inside it, and outside of every cell only past it.
const CONTAIN_EPSILON: f32 = 1.0e-4;

/// One room cell to build: the box `[-half, half]` under `affine`.
#[derive(Clone, Copy, Debug)]
pub struct RoomInput {
    pub affine: Affine3A,
    pub half: Vec3,
}

/// What one side of a portal opens into, as authored.
#[derive(Clone, Debug, PartialEq)]
pub enum SideInput {
    /// Found from the cells around it.
    Auto,
    /// These rooms (indices into the room list), and the outside when `outside` is set.
    Into { rooms: Vec<usize>, outside: bool },
}

/// One portal to build: the rectangle `size` in the local XY plane under `affine`. `sides[0]`
/// is behind its normal, `sides[1]` in front.
#[derive(Clone, Debug)]
pub struct PortalInput {
    pub affine: Affine3A,
    pub size: Vec2,
    pub sides: [SideInput; 2],
}

/// Why the graph couldn't be built. Indices are into the room and portal lists.
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    /// The room's box is flat, singular or not finite.
    Room(usize),
    /// The portal's rectangle is flat or not finite.
    Portal(usize),
    /// Two cells overlap by more than the tolerance, by this many meters.
    Overlap(usize, usize, f32),
    /// An automatic side has no certain answer.
    Side {
        portal: usize,
        side: usize,
        issue: Unresolved,
    },
    /// An explicit side opens into nothing, or into a room that doesn't exist.
    Into { portal: usize, side: usize },
}

/// A portal in the graph.
#[derive(Clone, Debug, PartialEq)]
pub struct PortalNode {
    pub aperture: Aperture,
    /// The nodes behind and in front of it.
    pub sides: [Vec<usize>; 2],
}

impl PortalNode {
    /// The same node on both sides: it joins nothing.
    pub fn is_ignored(&self) -> bool {
        self.sides[0]
            .iter()
            .all(|node| self.sides[1].contains(node))
            && self.sides[1]
                .iter()
                .all(|node| self.sides[0].contains(node))
    }
}

/// A way out of a node: through `portal` into `to`, crossing its plane along `outward`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub portal: usize,
    pub to: usize,
    pub outward: Vec3,
}

/// The validated rooms and portals.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub cells: Vec<Cell>,
    pub portals: Vec<PortalNode>,
    links: Vec<Vec<Link>>,
    pub tolerance: f32,
}

impl Graph {
    /// Validates `rooms` and links `portals` to them, with `tolerance` meters of slack where
    /// cells meet and where portals sit on them.
    pub fn build(
        rooms: &[RoomInput],
        portals: &[PortalInput],
        tolerance: f32,
    ) -> Result<Self, Problem> {
        let cells = rooms
            .iter()
            .enumerate()
            .map(|(index, room)| {
                Parallelepiped::from_affine(room.affine, room.half)
                    .map(Cell::new)
                    .ok_or(Problem::Room(index))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (a, cell) in cells.iter().enumerate() {
            for (b, other) in cells.iter().enumerate().skip(a + 1) {
                let depth = cell.shape.overlap(&other.shape);
                if depth > tolerance {
                    return Err(Problem::Overlap(a, b, depth));
                }
            }
        }

        let mut nodes = Vec::with_capacity(portals.len());
        for (index, portal) in portals.iter().enumerate() {
            let aperture =
                Aperture::from_affine(portal.affine, portal.size).ok_or(Problem::Portal(index))?;
            let mut sides: [Vec<usize>; 2] = Default::default();
            for (side, input) in portal.sides.iter().enumerate() {
                sides[side] = match input {
                    SideInput::Auto => {
                        let sign = if side == 0 { -1.0 } else { 1.0 };
                        let cell =
                            resolve(&aperture, sign, &cells, tolerance).map_err(|issue| {
                                Problem::Side {
                                    portal: index,
                                    side,
                                    issue,
                                }
                            })?;
                        vec![cell.map_or(OUTSIDE, |cell| cell + 1)]
                    }
                    SideInput::Into { rooms, outside } => {
                        if (rooms.is_empty() && !outside)
                            || rooms.iter().any(|&room| room >= cells.len())
                        {
                            return Err(Problem::Into {
                                portal: index,
                                side,
                            });
                        }
                        let mut nodes: Vec<usize> = rooms.iter().map(|room| room + 1).collect();
                        if *outside {
                            nodes.push(OUTSIDE);
                        }
                        nodes.sort_unstable();
                        nodes.dedup();
                        nodes
                    }
                };
            }
            nodes.push(PortalNode { aperture, sides });
        }

        let mut links = vec![Vec::new(); cells.len() + 1];
        for (index, portal) in nodes.iter().enumerate() {
            for side in 0..2 {
                let outward = if side == 0 {
                    portal.aperture.normal
                } else {
                    -portal.aperture.normal
                };
                for &from in &portal.sides[side] {
                    for &to in &portal.sides[1 - side] {
                        if from != to {
                            links[from].push(Link {
                                portal: index,
                                to,
                                outward,
                            });
                        }
                    }
                }
            }
        }
        Ok(Self {
            cells,
            portals: nodes,
            links,
            tolerance,
        })
    }

    /// Nodes: the outside and one per room.
    pub fn node_count(&self) -> usize {
        self.cells.len() + 1
    }

    /// The ways out of `node`.
    pub fn links(&self, node: usize) -> &[Link] {
        self.links.get(node).map_or(&[], Vec::as_slice)
    }

    /// The nodes `point` is in, into `out`: every cell within the tolerance of it (two where
    /// cells meet), and the outside unless a cell holds it.
    pub fn homes(&self, point: Vec3, out: &mut Vec<usize>) {
        out.clear();
        let mut held = false;
        for (index, cell) in self.cells.iter().enumerate() {
            let depth = cell.depth(point);
            if depth >= -self.tolerance {
                out.push(index + 1);
            }
            held |= depth >= -CONTAIN_EPSILON;
        }
        if !held {
            out.push(OUTSIDE);
        }
    }

    /// The nodes a sphere may reach into, into `out`: every cell it may touch, and the outside
    /// unless one cell holds the whole sphere. Walls between cells don't stop it.
    pub fn reach(&self, center: Vec3, radius: f32, out: &mut Vec<usize>) {
        out.clear();
        let mut held = false;
        for (index, cell) in self.cells.iter().enumerate() {
            if cell.touches_sphere(center, radius + self.tolerance) {
                out.push(index + 1);
                held |= cell.holds_sphere(center, radius);
            }
        }
        if !held {
            out.push(OUTSIDE);
        }
    }

    /// Whether `node` is one of the nodes [`Graph::reach`] gives for the sphere. A cell costs
    /// one test, the outside a pass over every cell.
    pub fn reaches(&self, node: usize, center: Vec3, radius: f32) -> bool {
        match node.checked_sub(1) {
            Some(cell) => self
                .cells
                .get(cell)
                .is_some_and(|cell| cell.touches_sphere(center, radius + self.tolerance)),
            // A cell that holds the sphere touches it too.
            None => !self
                .cells
                .iter()
                .any(|cell| cell.holds_sphere(center, radius)),
        }
    }
}
