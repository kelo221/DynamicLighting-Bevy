//! What one side of a portal opens into, found from the room cells around it. A side links only
//! when the answer is certain: one cell covering the whole opening within the tolerance, or no
//! cell near it at all. Anything else is reported, and the graph turns culling off rather than
//! guess.

use bevy::math::Vec3;

use super::cell::{Aperture, Cell, ray_span};

/// How far past a portal a cell counts as near it. A cell this close that doesn't reach the
/// portal is most likely a room drawn short of a thick wall, not open space.
pub const NEAR_MISS: f32 = 1.0;

/// Overlap below this is touching, not reaching into.
const REACH_EPSILON: f32 = 1.0e-4;

/// Why a side of a portal couldn't be linked. Cells are indices into the room list.
#[derive(Clone, Debug, PartialEq)]
pub enum Unresolved {
    /// Cells lie within [`NEAR_MISS`] of the opening, but none covers it from its plane.
    Uncovered(Vec<usize>),
    /// Several cells each cover the opening, so they overlap.
    Several(Vec<usize>),
}

/// The cell side `side` (+1 front, −1 back) of `aperture` opens into, `Ok(None)` for no cell
/// (the outside), or why it is unclear.
pub fn resolve(
    aperture: &Aperture,
    side: f32,
    cells: &[Cell],
    tolerance: f32,
) -> Result<Option<usize>, Unresolved> {
    let near = aperture.swept(side, tolerance, NEAR_MISS.max(tolerance * 2.0));
    let reaching: Vec<usize> = cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.shape.overlap(&near) > REACH_EPSILON)
        .map(|(index, _)| index)
        .collect();
    let covering: Vec<usize> = reaching
        .iter()
        .copied()
        .filter(|&index| covers(&cells[index], aperture, side, tolerance))
        .collect();
    match (covering.as_slice(), reaching.is_empty()) {
        ([one], _) => Ok(Some(*one)),
        ([], true) => Ok(None),
        ([], false) => Err(Unresolved::Uncovered(reaching)),
        _ => Err(Unresolved::Several(covering)),
    }
}

/// Whether `cell` takes the whole opening on side `side`: every corner, pulled in by the
/// tolerance, enters it within `tolerance` of the portal's plane, none enters it from more than
/// `tolerance` behind the plane, and all are inside together at some depth. The cell is convex,
/// so the whole opening is then inside it there, and every ray through the opening enters it.
pub fn covers(cell: &Cell, aperture: &Aperture, side: f32, tolerance: f32) -> bool {
    let along: Vec3 = aperture.normal * side;
    let mut deepest_entry = f32::NEG_INFINITY;
    let mut first_exit = f32::INFINITY;
    for corner in aperture.inset_corners(tolerance) {
        let Some((entry, exit)) = ray_span(&cell.planes, corner, along) else {
            return false;
        };
        if entry < -tolerance {
            return false;
        }
        deepest_entry = deepest_entry.max(entry);
        first_exit = first_exit.min(exit);
    }
    deepest_entry <= tolerance && deepest_entry < first_exit
}
