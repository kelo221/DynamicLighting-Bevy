//! CPU mirror of the shader side that reads baked data: Henry's `DynamicTriangle` (load,
//! set-active-light, shadow and bounce reads; `S/DynamicLighting.cginc` 608–962) and the
//! distance cube fetch. Tests run it over serialized words to prove the layout round-trips; the
//! WESL port must compute the same indices.

use bevy::math::Vec3;
use half::f16;

/// Henry clamps garbage light counts so a bad buffer can't hang the GPU.
pub const MAX_TRIANGLE_LIGHTS: u32 = 256;

/// A triangle's view into one mesh's `dynamic_triangles` words.
#[derive(Clone, Copy, Debug)]
pub struct TriangleView<'a> {
    words: &'a [u32],
    /// `(boundsX − 2, boundsY − 2, W)`.
    pub bounds: [u32; 3],
    light_data_offset: u32,
    pub light_count: u32,
    bounce_layout: bool,
}

/// The active light entry of a triangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightEntry {
    pub light_index: u32,
    pub shadow_offset: u32,
    pub bounce_offset: u32,
}

impl<'a> TriangleView<'a> {
    /// `DynamicTriangle.load`.
    pub fn load(words: &'a [u32], triangle_index: u32, bounce_layout: bool) -> Self {
        let offset = (triangle_index * 4) as usize;
        let light_data_offset = words[offset];
        let bounds = [words[offset + 1], words[offset + 2], words[offset + 3]];
        let light_count = words[light_data_offset as usize].min(MAX_TRIANGLE_LIGHTS);
        Self {
            words,
            bounds,
            light_data_offset: light_data_offset + 1,
            light_count,
            bounce_layout,
        }
    }

    /// `DynamicTriangle.set_active_light_index` for `k < light_count`.
    pub fn light(&self, k: u32) -> LightEntry {
        let stride = if self.bounce_layout { 3 } else { 2 };
        let offset = (self.light_data_offset + k * stride) as usize;
        LightEntry {
            light_index: self.words[offset],
            shadow_offset: self.words[offset + 1],
            bounce_offset: if self.bounce_layout {
                self.words[offset + 2]
            } else {
                0
            },
        }
    }

    /// The grid index of lightmap texel `(x, y)`, with the shader's wrapping `u32` math.
    fn grid_index(&self, x: u32, y: u32) -> u32 {
        let local_x = x.wrapping_sub(self.bounds[0]);
        let local_y = y.wrapping_sub(self.bounds[1]);
        local_y.wrapping_mul(self.bounds[2]).wrapping_add(local_x)
    }

    /// The shadow at lightmap texel `(x, y)` for a light with `bits` per texel (channel bits
    /// 20–22): how much light reaches it, 0 to 1. `None` when the entry has no shadow data
    /// (`is_occlusion_available` false).
    pub fn shadow_value(&self, entry: &LightEntry, x: u32, y: u32, bits: u32) -> Option<f32> {
        if entry.shadow_offset == 0 {
            return None;
        }
        let index = self.grid_index(x, y);
        let texels = 32 / bits;
        let mask = (1u32 << bits) - 1;
        let word = self.words[(entry.shadow_offset + index / texels) as usize];
        Some(((word >> ((index % texels) * bits)) & mask) as f32 / mask as f32)
    }

    /// [`Self::shadow_value`] of a one-bit (hard) shadow: whether the texel is lit.
    pub fn shadow_bit(&self, entry: &LightEntry, x: u32, y: u32) -> Option<bool> {
        self.shadow_value(entry, x, y, 1).map(|value| value > 0.5)
    }

    /// `bounce_sample` at lightmap texel `(x, y)`: the stored (square-rooted) value in `[0, 1]`.
    /// `bits_per_pixel` comes from the light's channel bits 17–19.
    pub fn bounce_texel(&self, entry: &LightEntry, x: u32, y: u32, bits_per_pixel: u32) -> f32 {
        let index = self.grid_index(x, y);
        let pixels = 32 / bits_per_pixel;
        let mask = (1u32 << bits_per_pixel) - 1;
        let value = self.words[(entry.bounce_offset + index / pixels) as usize];
        let byte = (value >> ((index % pixels) * bits_per_pixel)) & mask;
        byte as f32 / mask as f32
    }

    /// `bounce_texel_rgb`: a colored bounce texel's red, green and blue (square-rooted), each
    /// `bits_per_pixel` wide, read from the bit stream.
    pub fn bounce_texel_rgb(
        &self,
        entry: &LightEntry,
        x: u32,
        y: u32,
        bits_per_pixel: u32,
    ) -> Vec3 {
        let index = self.grid_index(x, y);
        let pixel_bits = 3 * bits_per_pixel;
        let mask = (1u32 << bits_per_pixel) - 1;
        let (offset, shift) = (index * pixel_bits / 32, index * pixel_bits % 32);
        let at = (entry.bounce_offset + offset) as usize;
        let mut word = self.words[at] >> shift;
        if shift + pixel_bits > 32 {
            word |= self.words[at + 1] << (32 - shift);
        }
        let channel = |c: u32| ((word >> (c * bits_per_pixel)) & mask) as f32 / mask as f32;
        Vec3::new(channel(0), channel(1), channel(2))
    }
}

/// Words per light in the distance cube buffer: 64² texels × 6 faces, two f16 per word.
pub const DISTANCE_CUBE_WORDS: u32 = 64 * 64 * 3;

/// `sample_distance_cube`'s buffer read: the stored distance of cube texel `index` (from the
/// index lookup cube) of the light in BVH slot `light`.
pub fn distance_cube_value(cubes: &[u32], light: u32, index: u32) -> f32 {
    let packed = cubes[(light * DISTANCE_CUBE_WORDS + (index >> 1)) as usize];
    f16::from_bits((packed >> ((index & 1) * 16)) as u16).to_f32()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::bits::TexelRect;
    use crate::bake::triangle_words::{TriangleLight, TriangleRecord, serialize_triangles};

    #[test]
    fn reads_back_what_the_builder_wrote() {
        let crop = TexelRect {
            min_x: 1,
            min_y: 0,
            max_x: 3,
            max_y: 2,
        };
        // Padded 7×7 grid; set the bit for lightmap texel (2, 1) = local (3, 3).
        let mut bits = crate::bake::bits::BitGrid::new(7, 7);
        bits.set(3, 3, true);
        let triangles = [TriangleRecord {
            crop,
            lights: vec![TriangleLight {
                light: 9,
                shadow: Some(bits.into_words()),
                bounce: Some(vec![0x0403_0201, 0x0807_0605]),
            }],
        }];
        let words = serialize_triangles(&triangles, true);
        let view = TriangleView::load(&words, 0, true);
        assert_eq!(view.light_count, 1);
        assert_eq!(view.bounds, [u32::MAX, u32::MAX - 1, 7]);
        let entry = view.light(0);
        assert_eq!(entry.light_index, 9);
        assert_eq!(view.shadow_bit(&entry, 2, 1), Some(true));
        assert_eq!(view.shadow_bit(&entry, 1, 1), Some(false));
        // The padding starts two texels before the crop rectangle: lightmap (-1, -2) is local
        // (0, 0) through the wrapping math.
        assert_eq!(view.shadow_bit(&entry, u32::MAX, u32::MAX - 1), Some(false));
        // 8 bits per pixel: pixel index 5 is byte 1 of the second word.
        let (x, y) = (5u32.wrapping_add(view.bounds[0]), view.bounds[1]);
        assert_eq!(view.bounce_texel(&entry, x, y, 8), 6.0 / 255.0);
    }

    #[test]
    fn soft_shadows_read_back_as_fractions() {
        let crop = TexelRect {
            min_x: 0,
            min_y: 0,
            max_x: 1,
            max_y: 1,
        };
        // Padded 6×6 grid of 4-bit levels; lightmap texel (0, 0) is local (2, 2).
        let mut grid = crate::bake::shadow_grid::ShadowGrid::new(6, 6, 4);
        grid.set(2, 2, 15);
        grid.set(3, 2, 5);
        let triangles = [TriangleRecord {
            crop,
            lights: vec![TriangleLight {
                light: 0,
                shadow: Some(grid.into_words()),
                bounce: None,
            }],
        }];
        let words = serialize_triangles(&triangles, false);
        let view = TriangleView::load(&words, 0, false);
        let entry = view.light(0);
        assert_eq!(view.shadow_value(&entry, 0, 0, 4), Some(1.0));
        assert_eq!(view.shadow_value(&entry, 1, 0, 4), Some(5.0 / 15.0));
        assert_eq!(view.shadow_value(&entry, 0, 1, 4), Some(0.0));
    }
}
