//! Picks which realtime lights fit the per-frame budget.

use bevy::math::Vec3;

/// A light that wants a realtime slot this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RealtimeCandidate<T> {
    pub key: T,
    pub position: Vec3,
    /// Radius of a sphere around the light containing everything it can reach.
    pub radius: f32,
}

/// Keeps at most `budget` candidates, in order. Over budget, the candidates are first sorted by
/// distance to `camera` so the nearest win; ties keep their original order. Candidates whose
/// sphere fails `visible` (the camera frustum test) never take a slot.
pub fn select_realtime<T: Copy>(
    mut candidates: Vec<RealtimeCandidate<T>>,
    camera: Option<Vec3>,
    budget: usize,
    visible: impl Fn(Vec3, f32) -> bool,
) -> Vec<RealtimeCandidate<T>> {
    if candidates.len() > budget
        && let Some(camera) = camera
    {
        candidates.sort_by(|a, b| {
            a.position
                .distance_squared(camera)
                .total_cmp(&b.position.distance_squared(camera))
        });
    }
    candidates
        .into_iter()
        .filter(|candidate| visible(candidate.position, candidate.radius))
        .take(budget)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(key: u32, x: f32) -> RealtimeCandidate<u32> {
        RealtimeCandidate {
            key,
            position: Vec3::new(x, 0.0, 0.0),
            radius: 1.0,
        }
    }

    fn keys(selected: &[RealtimeCandidate<u32>]) -> Vec<u32> {
        selected.iter().map(|candidate| candidate.key).collect()
    }

    #[test]
    fn under_budget_keeps_everything_in_order() {
        let all = vec![candidate(0, 9.0), candidate(1, 1.0), candidate(2, 5.0)];
        let selected = select_realtime(all, Some(Vec3::ZERO), 8, |_, _| true);
        assert_eq!(keys(&selected), [0, 1, 2]);
    }

    #[test]
    fn over_budget_keeps_the_nearest() {
        let all = vec![candidate(0, 9.0), candidate(1, 1.0), candidate(2, 5.0)];
        let selected = select_realtime(all, Some(Vec3::ZERO), 2, |_, _| true);
        assert_eq!(keys(&selected), [1, 2]);
    }

    #[test]
    fn invisible_lights_never_take_a_slot() {
        let all = vec![candidate(0, -3.0), candidate(1, 1.0), candidate(2, 5.0)];
        let selected = select_realtime(all, Some(Vec3::ZERO), 2, |position, _| position.x > 0.0);
        assert_eq!(keys(&selected), [1, 2]);
    }

    #[test]
    fn zero_budget_selects_nothing() {
        let selected = select_realtime(vec![candidate(0, 1.0)], None, 0, |_, _| true);
        assert!(selected.is_empty());
    }
}
