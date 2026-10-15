//! Henry's `CompressBounceLighting*BitsPerPixel`: bounce values square-rooted (detail in dark
//! shades), quantized to 4–8 bits with a little dither below 8 bits, and packed
//! `floor(32 / bits)` pixels per word, pixel `j` of a word at bit `j · bits`.
//!
//! The shader decodes with `(word >> ((i % n) · bits)) & mask`, divides by the mask and squares.

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
    let max = ((1u32 << bits) - 1) as f32;
    let amplitude = dither_amplitude(compression);
    values
        .chunks(per_word)
        .map(|chunk| {
            chunk.iter().enumerate().fold(0u32, |word, (j, &value)| {
                let mut color = if value.is_nan() {
                    0.0
                } else {
                    value.clamp(0.0, 1.0)
                };
                if amplitude > 0.0 && color > 0.001 {
                    color = (color + rng.next_f32() * amplitude).min(1.0);
                }
                let quantized = (color.sqrt() * max) as u32;
                word | quantized << (j as u32 * bits)
            })
        })
        .collect()
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
}
