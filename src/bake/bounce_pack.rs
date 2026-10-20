//! Henry's `CompressBounceLighting*BitsPerPixel`: bounce values square-rooted (detail in dark
//! shades), quantized to 4–8 bits with a little dither below 8 bits, and packed
//! `floor(32 / bits)` pixels per word, pixel `j` of a word at bit `j · bits`.
//!
//! The shader decodes with `(word >> ((i % n) · bits)) & mask`, divides by the mask and squares.
//!
//! Colored bounce ([`pack_bounce_rgb`]) stores red, green and blue of a pixel side by side,
//! `3 · bits` wide, as one bit stream: pixel `i` starts at bit `3 · bits · i` and may continue
//! in the next word, so no bits are left over (a quarter of every word at 8 bits).

use bevy::math::Vec3;

use crate::effects::EffectRng;
use crate::light::BounceCompression;

/// Henry's dither amplitude per precision: 0 for 8 bits, else a uniform `[0, amplitude)` added to
/// values above 0.001.
pub fn dither_amplitude(compression: BounceCompression) -> f32 {
    match compression {
        BounceCompression::Bits8 => 0.0,
        BounceCompression::Bits6 => 0.004,
        BounceCompression::Bits5 => 0.008,
        BounceCompression::Bits4 => 0.02,
    }
}

/// Packs `values` (linear bounce, clamped to `[0, 1]`). `rng` supplies the dither.
pub fn pack_bounce(
    values: &[f32],
    compression: BounceCompression,
    rng: &mut EffectRng,
) -> Vec<u32> {
    let bits = compression.bits_per_pixel();
    let per_word = (32 / bits) as usize;
    values
        .chunks(per_word)
        .map(|chunk| {
            chunk.iter().enumerate().fold(0u32, |word, (j, &value)| {
                word | quantize(value, compression, rng) << (j as u32 * bits)
            })
        })
        .collect()
}

/// Packs colored `values` (linear, each channel clamped to `[0, 1]`), one dither draw per
/// channel.
pub fn pack_bounce_rgb(
    values: &[Vec3],
    compression: BounceCompression,
    rng: &mut EffectRng,
) -> Vec<u32> {
    let bits = compression.bits_per_pixel();
    let pixel_bits = 3 * bits;
    let mut words = vec![0u32; (values.len() * pixel_bits as usize).div_ceil(32)];
    for (i, value) in values.iter().enumerate() {
        let pixel = value
            .to_array()
            .iter()
            .enumerate()
            .fold(0u32, |pixel, (c, &channel)| {
                pixel | quantize(channel, compression, rng) << (c as u32 * bits)
            });
        let offset = i * pixel_bits as usize;
        let (word, shift) = (offset / 32, (offset % 32) as u32);
        words[word] |= pixel << shift;
        if shift + pixel_bits > 32 {
            words[word + 1] |= pixel >> (32 - shift);
        }
    }
    words
}

/// One value square-rooted and quantized, with Henry's dither below 8 bits.
fn quantize(value: f32, compression: BounceCompression, rng: &mut EffectRng) -> u32 {
    let max = ((1u32 << compression.bits_per_pixel()) - 1) as f32;
    let amplitude = dither_amplitude(compression);
    let mut color = if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    };
    if amplitude > 0.0 && color > 0.001 {
        color = (color + rng.next_f32() * amplitude).min(1.0);
    }
    (color.sqrt() * max) as u32
}

#[cfg(test)]
/// Inverse of [`pack_bounce`] for one pixel, as the shader computes it (after squaring).
pub fn unpack_bounce(words: &[u32], index: usize, compression: BounceCompression) -> f32 {
    let bits = compression.bits_per_pixel();
    let per_word = (32 / bits) as usize;
    let mask = (1u32 << bits) - 1;
    let value = (words[index / per_word] >> ((index % per_word) as u32 * bits)) & mask;
    let stored = value as f32 / mask as f32;
    stored * stored
}

#[cfg(test)]
/// Inverse of [`pack_bounce_rgb`] for one pixel, as the shader computes it (after squaring).
pub fn unpack_bounce_rgb(words: &[u32], index: usize, compression: BounceCompression) -> Vec3 {
    let bits = compression.bits_per_pixel();
    let pixel_bits = 3 * bits;
    let mask = (1u32 << bits) - 1;
    let offset = index * pixel_bits as usize;
    let (word, shift) = (offset / 32, (offset % 32) as u32);
    let mut pixel = words[word] >> shift;
    if shift + pixel_bits > 32 {
        pixel |= words[word + 1] << (32 - shift);
    }
    let channel = |c: u32| {
        let stored = ((pixel >> (c * bits)) & mask) as f32 / mask as f32;
        stored * stored
    };
    Vec3::new(channel(0), channel(1), channel(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [BounceCompression; 4] = [
        BounceCompression::Bits8,
        BounceCompression::Bits6,
        BounceCompression::Bits5,
        BounceCompression::Bits4,
    ];

    #[test]
    fn pixels_per_word_follow_henry() {
        let values = vec![1.0; 9];
        let counts: Vec<usize> = ALL
            .iter()
            .map(|&c| pack_bounce(&values, c, &mut EffectRng::default()).len())
            .collect();
        // 4, 5, 6 and 8 pixels per word.
        assert_eq!(counts, [3, 2, 2, 2]);
        let words = pack_bounce(
            &[1.0, 0.0, 1.0],
            BounceCompression::Bits8,
            &mut EffectRng::default(),
        );
        assert_eq!(words, [0x00FF_00FF]);
        let words = pack_bounce(
            &[1.0; 5],
            BounceCompression::Bits6,
            &mut EffectRng::default(),
        );
        assert_eq!(words, [(1 << 30) - 1]);
    }

    #[test]
    fn values_round_trip_within_quantization() {
        let values: Vec<f32> = (0..64).map(|i| i as f32 / 63.0).collect();
        for compression in ALL {
            let words = pack_bounce(&values, compression, &mut EffectRng::default());
            let max = ((1u32 << compression.bits_per_pixel()) - 1) as f32;
            for (i, &value) in values.iter().enumerate() {
                let decoded = unpack_bounce(&words, i, compression).sqrt();
                // Truncation loses up to one step; dither adds at most its amplitude.
                let dithered = (value + dither_amplitude(compression)).min(1.0).sqrt();
                assert!(decoded <= dithered + 1e-6, "{compression:?} {value}");
                assert!(
                    decoded >= value.sqrt() - 1.0 / max - 1e-6,
                    "{compression:?} {value}"
                );
            }
        }
    }

    #[test]
    fn black_and_invalid_values_stay_black() {
        for compression in ALL {
            let values = [0.0, f32::NAN, -1.0];
            let words = pack_bounce(&values, compression, &mut EffectRng::default());
            for i in 0..values.len() {
                assert_eq!(unpack_bounce(&words, i, compression), 0.0);
            }
        }
    }

    #[test]
    fn colored_pixels_pack_side_by_side() {
        let values = [Vec3::new(1.0, 0.0, 0.25), Vec3::ONE];
        let words = pack_bounce_rgb(&values, BounceCompression::Bits8, &mut EffectRng::default());
        // 24-bit pixels at 8 bits, red low, then green, then blue (sqrt 0.25 = 0.5); the second
        // starts at bit 24 and runs on into the next word.
        assert_eq!(words, [0xFF7F_00FF, 0xFFFF]);
        let words = pack_bounce_rgb(&values, BounceCompression::Bits5, &mut EffectRng::default());
        assert_eq!(words.len(), 1, "two 15-bit pixels fit a word at 5 bits");
        let words = pack_bounce_rgb(
            &[Vec3::ONE; 16],
            BounceCompression::Bits6,
            &mut EffectRng::default(),
        );
        assert_eq!(words.len(), 9, "sixteen 18-bit pixels take 288 bits");
        for compression in ALL {
            let values: Vec<Vec3> = (0..9)
                .map(|i| Vec3::new(i as f32 / 8.0, 1.0 - i as f32 / 8.0, 0.5))
                .collect();
            let words = pack_bounce_rgb(&values, compression, &mut EffectRng::default());
            let step = 1.0 / ((1u32 << compression.bits_per_pixel()) - 1) as f32;
            for (i, value) in values.iter().enumerate() {
                let decoded = unpack_bounce_rgb(&words, i, compression);
                for c in 0..3 {
                    let (stored, wanted) = (decoded[c].sqrt(), value[c].sqrt());
                    let dithered = (value[c] + dither_amplitude(compression)).min(1.0).sqrt();
                    assert!(stored >= wanted - step - 1e-6 && stored <= dithered + 1e-6);
                }
            }
        }
    }
}
