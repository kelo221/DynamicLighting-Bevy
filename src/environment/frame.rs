//! A sky light's orientation and strength as the shaders see them, read back from its packed
//! light record.

use bevy::math::Vec3;

use crate::light::LightType;
use crate::shared::{ChannelFlags, GpuLight, GpuLightGlobals};

/// The axes of a sky light's image in world space, and the light's color times intensity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyFrame {
    /// The entity's local +X, +Y and +Z.
    pub right: Vec3,
    pub up: Vec3,
    pub back: Vec3,
    /// Linear RGB the image is multiplied by.
    pub tint: Vec3,
}

impl SkyFrame {
    /// The frame of a packed sky light: its record holds the entity's forward (−Z) and up axes.
    pub fn of(light: &GpuLight) -> Self {
        let forward = Vec3::from(light.forward);
        let up = Vec3::from(light.up);
        Self {
            right: forward.cross(up),
            up,
            back: -forward,
            tint: Vec3::from(light.color) * light.intensity,
        }
    }

    /// A world direction in the image's frame (`sky_local` in `sky.wesl`).
    pub fn to_local(&self, direction: Vec3) -> Vec3 {
        Vec3::new(
            direction.dot(self.right),
            direction.dot(self.up),
            direction.dot(self.back),
        )
    }
}

/// The first enabled sky light among a light buffer's baked distant lights, then its realtime
/// ones, which the background shows.
pub fn first_sky(globals: &GpuLightGlobals, lights: &[GpuLight]) -> Option<SkyFrame> {
    let baked = globals.baked_count as usize;
    let baked_distant = baked.saturating_sub(globals.baked_distant as usize)..baked;
    let realtime_distant = baked..baked + globals.realtime_distant as usize;
    [baked_distant, realtime_distant]
        .into_iter()
        .filter_map(|range| lights.get(range))
        .flatten()
        .find_map(|light| {
            let kind = ChannelFlags::unpack(light.channel).light_type;
            let sky = kind == LightType::Sky as u32 && light.radius_sqr >= 0.0;
            sky.then(|| SkyFrame::of(light))
        })
}

#[cfg(test)]
mod tests {
    use bevy::math::Quat;

    use super::*;
    use crate::light::{DynamicLight, LightPose, PackContext, pack};

    fn packed_sky(rotation: Quat) -> GpuLight {
        let pose = LightPose {
            position: Vec3::ZERO,
            rotation,
        };
        pack(&DynamicLight::sky(), pose, PackContext::default())
    }

    #[test]
    fn an_unturned_sky_keeps_world_axes() {
        let frame = SkyFrame::of(&packed_sky(Quat::IDENTITY));
        let d = Vec3::new(0.3, -0.5, 0.8).normalize();
        assert!(frame.to_local(d).abs_diff_eq(d, 1e-6));
        assert!(
            frame.tint.abs_diff_eq(Vec3::ONE, 1e-6),
            "white at intensity 1"
        );
    }

    #[test]
    fn turning_the_entity_turns_the_sky() {
        let rotation = Quat::from_rotation_y(0.7) * Quat::from_rotation_x(0.2);
        let frame = SkyFrame::of(&packed_sky(rotation));
        let local = Vec3::new(0.1, 0.4, -0.9).normalize();
        assert!(frame.to_local(rotation * local).abs_diff_eq(local, 1e-5));
    }

    #[test]
    fn the_background_finds_the_baked_or_realtime_sky() {
        let sky = packed_sky(Quat::IDENTITY);
        let other = GpuLight::default();
        let globals = GpuLightGlobals {
            baked_count: 1,
            realtime_count: 2,
            realtime_distant: 2,
            ..Default::default()
        };
        assert!(first_sky(&globals, &[sky, other, sky]).is_some());
        assert!(
            first_sky(&globals, &[sky, other, other]).is_none(),
            "a baked slot that isn't distant"
        );
        let baked = GpuLightGlobals {
            baked_distant: 1,
            ..globals
        };
        assert!(first_sky(&baked, &[sky, other, other]).is_some());
        let disabled = GpuLight {
            radius_sqr: -1.0,
            ..sky
        };
        assert!(first_sky(&globals, &[other, disabled, other]).is_none());
    }
}
