//! The volumetric fog authoring component.

use bevy::prelude::*;

/// Fills a volume around a `DynamicLight` with fog in the light's color, drawn by the fog
/// post-process. The fog takes the light's intensity effect.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component, Default, Debug, PartialEq, Clone)]
pub struct VolumetricLight {
    pub shape: VolumetricShape,
    /// Size of the volume in meters; the fog is completely gone at its edge.
    pub radius: f32,
    /// How hard the fog is to see through, at least 1. Thick fog looks like a solid color.
    pub thickness: f32,
    /// Opacity of the fog, from 0 (invisible) to 1.
    pub intensity: f32,
    /// Distance in meters within which objects inside the fog stay clearly visible.
    pub visibility: f32,
}

impl Default for VolumetricLight {
    fn default() -> Self {
        Self {
            shape: VolumetricShape::Sphere,
            radius: 4.0,
            thickness: 1.0,
            intensity: 0.75,
            visibility: 2.0,
        }
    }
}

/// The shape of a fog volume. Discriminants are the values the fog shader reads.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
#[repr(u32)]
pub enum VolumetricShape {
    #[default]
    Sphere = 1,
    /// A box of half-size `radius`, stretched by the light's scale and turned with the light.
    Box = 2,
    /// A cone along the light's forward axis, as wide as the light's `outer_cutoff`.
    ConeZ = 3,
    /// A cone along the light's up axis, as wide as the light's `outer_cutoff`.
    ConeY = 4,
    /// A rectangular shaft `radius` long along the light's forward axis: it starts as the shape of
    /// a rectangle or tube light (a point for other types) and widens by the light's
    /// `outer_cutoff` on every side. A ceiling panel's light falling through haze.
    Frustum = 5,
}
