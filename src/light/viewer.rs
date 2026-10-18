//! The camera that realtime lights, shadow cubemaps and the sun's cascades are chosen and fitted
//! for.

use bevy::prelude::*;

/// Marks the camera the realtime lights are chosen for, the shadow cubemaps go to and the sun's
/// cascades cover. Without one, the highest-order active `Camera3d` is used, so 2D and UI
/// cameras never steer them.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Component, Debug, Clone, Default, PartialEq)]
pub struct LightingViewer;

/// How a camera ranks as the viewer, the highest first; `None` for cameras that can't be it:
/// inactive ones, and those neither marked [`LightingViewer`] nor 3D. Marked cameras come
/// before the others, then higher `order` before lower.
pub fn viewer_rank(
    active: bool,
    order: isize,
    three_d: bool,
    marked: bool,
) -> Option<(bool, isize)> {
    (active && (marked || three_d)).then_some((marked, order))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The index of the camera chosen among (active, order, 3D, marked).
    fn chosen(cameras: &[(bool, isize, bool, bool)]) -> Option<usize> {
        cameras
            .iter()
            .enumerate()
            .filter_map(|(index, &(active, order, three_d, marked))| {
                viewer_rank(active, order, three_d, marked).map(|rank| (rank, index))
            })
            .max_by_key(|(rank, _)| *rank)
            .map(|(_, index)| index)
    }

    #[test]
    fn a_ui_camera_on_top_doesnt_steer_the_lights() {
        let world = (true, 0, true, false);
        let ui = (true, 1, false, false);
        assert_eq!(chosen(&[world, ui]), Some(0));
        assert_eq!(chosen(&[ui]), None);
    }

    #[test]
    fn the_marked_camera_wins_and_otherwise_the_highest_order() {
        let low = (true, 0, true, false);
        let high = (true, 2, true, false);
        let marked = (true, -1, true, true);
        assert_eq!(chosen(&[low, high]), Some(1));
        assert_eq!(chosen(&[low, high, marked]), Some(2));
        let inactive = (false, -1, true, true);
        assert_eq!(chosen(&[low, inactive]), Some(0));
        let marked_2d = (true, 5, false, true);
        assert_eq!(
            chosen(&[high, marked_2d]),
            Some(1),
            "marking makes it count"
        );
    }
}
