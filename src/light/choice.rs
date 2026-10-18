//! Stable choices among lights: which light wins where only one of its kind can be used, and the
//! order distant lights are packed in. Ties never fall to entity ids or query order, which
//! change between runs: the brightest light comes first, then the one first by `Name` path, then
//! the one first by its transform and color.

use core::cmp::Ordering;

use bevy::color::ColorToComponents;

use super::component::DynamicLight;
use super::pack::LightPose;

/// What a light is ranked by.
#[derive(Clone, Debug, PartialEq)]
pub struct LightRank {
    intensity: f32,
    path: Vec<String>,
    /// Position, rotation and linear color, bit by bit.
    content: [u32; 10],
}

impl LightRank {
    /// The rank of `light` at `pose` whose entity has the `Name` path `path`.
    pub fn new(light: &DynamicLight, pose: LightPose, path: Vec<String>) -> Self {
        let color = light.color.to_linear().to_f32_array_no_alpha();
        let values = pose
            .position
            .to_array()
            .into_iter()
            .chain(pose.rotation.to_array())
            .chain(color);
        let mut content = [0; 10];
        for (slot, value) in content.iter_mut().zip(values) {
            *slot = value.to_bits();
        }
        Self {
            intensity: light.intensity,
            path,
            content,
        }
    }

    /// Brightest first, then by name path, then by transform and color.
    pub fn order(&self, other: &Self) -> Ordering {
        other
            .intensity
            .total_cmp(&self.intensity)
            .then_with(|| self.path.cmp(&other.path))
            .then_with(|| self.content.cmp(&other.content))
    }
}

/// The key of the first-ranked candidate.
pub fn first_ranked<K>(candidates: impl IntoIterator<Item = (K, LightRank)>) -> Option<K> {
    candidates
        .into_iter()
        .min_by(|(_, a), (_, b)| a.order(b))
        .map(|(key, _)| key)
}

#[cfg(test)]
mod tests {
    use bevy::color::Color;
    use bevy::math::{Quat, Vec3};

    use super::*;

    fn rank(intensity: f32, name: &str, x: f32) -> LightRank {
        let light = DynamicLight {
            intensity,
            ..DynamicLight::sun()
        };
        let pose = LightPose {
            position: Vec3::new(x, 0.0, 0.0),
            rotation: Quat::IDENTITY,
        };
        LightRank::new(&light, pose, vec![name.to_string()])
    }

    #[test]
    fn the_brightest_wins_then_the_name_path() {
        let lights = [(1, rank(2.0, "b", 0.0)), (2, rank(3.0, "c", 0.0))];
        assert_eq!(first_ranked(lights.clone()), Some(2));
        let tied = [(1, rank(3.0, "c", 0.0)), (2, rank(3.0, "a", 0.0))];
        assert_eq!(first_ranked(tied), Some(2), "ties go by name path");
        assert_eq!(first_ranked(lights[..1].to_vec()), Some(1));
        assert_eq!(first_ranked::<u32>([]), None);
    }

    #[test]
    fn lights_alike_in_name_and_intensity_go_by_their_content_in_any_order() {
        let (a, b) = (rank(1.0, "", 0.0), rank(1.0, "", 2.0));
        let first = first_ranked([(1, a.clone()), (2, b.clone())]);
        assert_eq!(first, first_ranked([(2, b), (1, a.clone())]));
        let red = LightRank::new(
            &DynamicLight {
                color: Color::srgb(1.0, 0.0, 0.0),
                intensity: 1.0,
                ..DynamicLight::sun()
            },
            LightPose {
                position: Vec3::ZERO,
                rotation: Quat::IDENTITY,
            },
            vec![String::new()],
        );
        assert_ne!(red.order(&a), Ordering::Equal, "the color counts");
    }
}
