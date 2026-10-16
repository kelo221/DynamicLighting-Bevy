//! The GPU light record and the bit layout of its `channel` word.

use bytemuck::{Pod, Zeroable};

/// Size in bytes of one [`GpuLight`]. The WESL `GpuLight` in `shaders/types.wesl` must match.
pub const GPU_LIGHT_SIZE: usize = 112;

/// One light as the shaders read it, seven 16-byte rows.
///
/// `bounce_color` comes before `falloff` in the last row: WGSL aligns `vec3<f32>` to 16 bytes,
/// so the opposite order would grow the record to 128 bytes.
///
/// The general-purpose floats depend on the light type: cutoff cosines for spot and discoball
/// lights, wave time and frequency for the wave-like types, and so on.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuLight {
    pub position: [f32; 3],
    /// Squared radius; negative disables the light in every shader path.
    pub radius_sqr: f32,
    /// Flag bits, see [`ChannelFlags`].
    pub channel: u32,
    pub intensity: f32,
    pub gp1: f32,
    pub gp2: f32,
    /// Linear RGB.
    pub color: [f32; 3],
    pub gp3: f32,
    pub up: [f32; 3],
    pub shimmer_scale: f32,
    pub forward: [f32; 3],
    pub shimmer_modifier: f32,
    pub volumetric_intensity: f32,
    pub volumetric_visibility: f32,
    /// Spot lights: the cookie's layer. Area lights, which take no cookies: the bits of the
    /// `f32` offset along `up` from the shape's center to the record's shadow point.
    pub cookie_index: u32,
    pub shadow_cubemap_index: u32,
    /// Linear RGB.
    pub bounce_color: [f32; 3],
    pub falloff: f32,
}

impl GpuLight {
    /// A slot every shader path skips, used for baked lights that were removed or moved away
    /// from where they were baked.
    pub const DISABLED: Self = Self {
        position: [0.0; 3],
        radius_sqr: -1.0,
        channel: 0,
        intensity: 0.0,
        gp1: 0.0,
        gp2: 0.0,
        color: [0.0; 3],
        gp3: 0.0,
        up: [0.0; 3],
        shimmer_scale: 0.0,
        forward: [0.0; 3],
        shimmer_modifier: 0.0,
        volumetric_intensity: 0.0,
        volumetric_visibility: 0.0,
        cookie_index: 0,
        shadow_cubemap_index: 0,
        bounce_color: [0.0; 3],
        falloff: 0.0,
    };
}

/// The fields packed into [`GpuLight::channel`]. Bits 0–4 and 12–14 are unused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelFlags {
    /// Bit 5: evaluated without baked shadows.
    pub realtime: bool,
    /// Bits 6–9: the light type discriminant.
    pub light_type: u32,
    /// Bits 10–11: the shimmer discriminant.
    pub shimmer: u32,
    /// Bit 15: a realtime shadow cubemap is bound at `shadow_cubemap_index`.
    pub shadow_cubemap: bool,
    /// Bit 16: a cookie texture is bound at `cookie_index`.
    pub cookie: bool,
    /// Bits 17–19: bounce bits per pixel, 1 to 8, stored minus one.
    pub bounce_bits_per_pixel: u32,
    /// Bits 20–31: the shadow points of a baked area light whose records fill consecutive
    /// slots share their first slot plus one here, so the shaders shade the light once and only
    /// add up the points' shadows. 0 for every other record.
    pub group: u32,
}

impl ChannelFlags {
    pub const REALTIME: u32 = 1 << 5;
    pub const TYPE_SHIFT: u32 = 6;
    pub const TYPE_MASK: u32 = 0b1111 << Self::TYPE_SHIFT;
    pub const SHIMMER_SHIFT: u32 = 10;
    pub const SHIMMER_MASK: u32 = 0b11 << Self::SHIMMER_SHIFT;
    pub const SHADOW_CUBEMAP: u32 = 1 << 15;
    pub const COOKIE: u32 = 1 << 16;
    pub const BOUNCE_SHIFT: u32 = 17;
    pub const BOUNCE_MASK: u32 = 0b111 << Self::BOUNCE_SHIFT;
    pub const GROUP_SHIFT: u32 = 20;
    /// The largest [`Self::group`]: slot 4094 plus one.
    pub const MAX_GROUP: u32 = (1 << 12) - 1;

    /// Packs the flags; out-of-range values are masked to their field width.
    pub fn pack(self) -> u32 {
        let mut channel = (self.light_type << Self::TYPE_SHIFT) & Self::TYPE_MASK;
        channel |= (self.shimmer << Self::SHIMMER_SHIFT) & Self::SHIMMER_MASK;
        if self.realtime {
            channel |= Self::REALTIME;
        }
        if self.shadow_cubemap {
            channel |= Self::SHADOW_CUBEMAP;
        }
        if self.cookie {
            channel |= Self::COOKIE;
        }
        let bounce = self.bounce_bits_per_pixel.clamp(1, 8) - 1;
        channel |= (bounce << Self::BOUNCE_SHIFT) & Self::BOUNCE_MASK;
        channel | ((self.group & Self::MAX_GROUP) << Self::GROUP_SHIFT)
    }

    pub fn unpack(channel: u32) -> Self {
        Self {
            realtime: channel & Self::REALTIME != 0,
            light_type: (channel & Self::TYPE_MASK) >> Self::TYPE_SHIFT,
            shimmer: (channel & Self::SHIMMER_MASK) >> Self::SHIMMER_SHIFT,
            shadow_cubemap: channel & Self::SHADOW_CUBEMAP != 0,
            cookie: channel & Self::COOKIE != 0,
            bounce_bits_per_pixel: 1 + ((channel & Self::BOUNCE_MASK) >> Self::BOUNCE_SHIFT),
            group: channel >> Self::GROUP_SHIFT,
        }
    }
}

#[cfg(test)]
mod tests {
    use core::mem::offset_of;

    use super::*;
    use crate::shared::wesl_check;

    const FIELD_OFFSETS: [(&str, usize); 18] = [
        ("position", offset_of!(GpuLight, position)),
        ("radius_sqr", offset_of!(GpuLight, radius_sqr)),
        ("channel", offset_of!(GpuLight, channel)),
        ("intensity", offset_of!(GpuLight, intensity)),
        ("gp1", offset_of!(GpuLight, gp1)),
        ("gp2", offset_of!(GpuLight, gp2)),
        ("color", offset_of!(GpuLight, color)),
        ("gp3", offset_of!(GpuLight, gp3)),
        ("up", offset_of!(GpuLight, up)),
        ("shimmer_scale", offset_of!(GpuLight, shimmer_scale)),
        ("forward", offset_of!(GpuLight, forward)),
        ("shimmer_modifier", offset_of!(GpuLight, shimmer_modifier)),
        (
            "volumetric_intensity",
            offset_of!(GpuLight, volumetric_intensity),
        ),
        (
            "volumetric_visibility",
            offset_of!(GpuLight, volumetric_visibility),
        ),
        ("cookie_index", offset_of!(GpuLight, cookie_index)),
        (
            "shadow_cubemap_index",
            offset_of!(GpuLight, shadow_cubemap_index),
        ),
        ("bounce_color", offset_of!(GpuLight, bounce_color)),
        ("falloff", offset_of!(GpuLight, falloff)),
    ];

    #[test]
    fn rust_layout_is_seven_rows_of_sixteen_bytes() {
        assert_eq!(size_of::<GpuLight>(), GPU_LIGHT_SIZE);
        let expected = [
            0, 12, 16, 20, 24, 28, 32, 44, 48, 60, 64, 76, 80, 84, 88, 92, 96, 108,
        ];
        let actual = FIELD_OFFSETS.map(|(_, offset)| offset);
        assert_eq!(actual, expected);
    }

    #[test]
    fn wesl_struct_matches_the_rust_struct() {
        let module = wesl_check::compile_shared_types();
        let layout = wesl_check::struct_layout(&module, "GpuLight");
        assert_eq!(layout.size as usize, GPU_LIGHT_SIZE);
        let names: Vec<_> = FIELD_OFFSETS.iter().map(|(name, _)| *name).collect();
        let wesl_names: Vec<_> = layout
            .members
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(wesl_names, names);
        for ((name, rust), (_, wesl)) in FIELD_OFFSETS.iter().zip(&layout.members) {
            assert_eq!(*wesl as usize, *rust, "offset of {name}");
        }
    }

    #[test]
    fn channel_bits_round_trip() {
        let flags = ChannelFlags {
            realtime: true,
            light_type: 7,
            shimmer: 2,
            shadow_cubemap: true,
            cookie: true,
            bounce_bits_per_pixel: 6,
            group: ChannelFlags::MAX_GROUP,
        };
        let channel = flags.pack();
        assert_eq!(channel & 0b11111, 0, "bits 0-4 stay unused");
        assert_eq!(channel & (0b111 << 12), 0, "bits 12-14 stay unused");
        assert_eq!(ChannelFlags::unpack(channel), flags);
    }

    #[test]
    fn channel_bits_match_the_shader_masks() {
        assert_eq!(ChannelFlags::REALTIME, 32);
        assert_eq!(ChannelFlags::TYPE_MASK, 960);
        assert_eq!(ChannelFlags::SHIMMER_MASK, 3072);
        assert_eq!(ChannelFlags::SHADOW_CUBEMAP, 32_768);
        assert_eq!(ChannelFlags::COOKIE, 65_536);
        assert_eq!(ChannelFlags::BOUNCE_MASK, 917_504);
        let spot = ChannelFlags {
            light_type: 1,
            bounce_bits_per_pixel: 8,
            ..Default::default()
        };
        assert_eq!(spot.pack(), (1 << 6) | (7 << 17));
    }

    #[test]
    fn out_of_range_fields_are_masked() {
        let flags = ChannelFlags {
            light_type: 0xFFFF,
            shimmer: 0xFF,
            bounce_bits_per_pixel: 0,
            group: u32::MAX,
            ..Default::default()
        };
        let unpacked = ChannelFlags::unpack(flags.pack());
        assert_eq!(unpacked.light_type, 15);
        assert_eq!(unpacked.shimmer, 3);
        assert_eq!(unpacked.bounce_bits_per_pixel, 1);
        assert_eq!(unpacked.group, ChannelFlags::MAX_GROUP);
    }

    #[test]
    fn disabled_slot_has_negative_radius() {
        const { assert!(GpuLight::DISABLED.radius_sqr < 0.0) };
        assert_eq!(
            bytemuck::bytes_of(&GpuLight::DISABLED).len(),
            GPU_LIGHT_SIZE
        );
    }
}
