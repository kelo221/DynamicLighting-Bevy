//! Tests of the shadow filter mirror: row reads against texel reads at every bit depth.

use super::*;
use crate::effects::EffectRng;

/// The filters as they read one texel at a time before the row reads.
fn texel_by_texel(bits: &ShadowTexels, pixel: Vec2, quality: u32) -> f32 {
    let corner = bits.corner(pixel);
    let f = (pixel - 0.5).fract();
    let levels = match quality {
        QUALITY_INTEGRATED => {
            let t = |x, y| bits.texel(corner + IVec2::new(x, y));
            let c = Vec2::new(t(0, 0), t(0, 1)).lerp(Vec2::new(t(1, 0), t(1, 1)), f.x);
            c.x + (c.y - c.x) * f.y
        }
        QUALITY_LOW => {
            let uv = pixel
                .max(Vec2::ZERO)
                .as_uvec2()
                .wrapping_sub(bits.origin)
                .as_ivec2();
            let mut sum = 0.0;
            for y in -1..=1 {
                for x in -1..=1 {
                    sum += bits.texel(uv + IVec2::new(x, y));
                }
            }
            sum / 9.0
        }
        _ => {
            let weights = [1.0, 4.0, 6.0, 4.0, 1.0];
            let mut map = [[0.0f32; 4]; 4];
            for (y, line) in map.iter_mut().enumerate() {
                for (x, value) in line.iter_mut().enumerate() {
                    let texel = corner + IVec2::new(x as i32 - 1, y as i32 - 1);
                    if quality < QUALITY_HIGH {
                        *value = bits.texel(texel);
                        continue;
                    }
                    let mut sum = 0.0;
                    for (j, wy) in weights.iter().enumerate() {
                        for (i, wx) in weights.iter().enumerate() {
                            let tap = texel + IVec2::new(i as i32 - 2, j as i32 - 2);
                            sum += wx * wy * bits.texel(tap);
                        }
                    }
                    *value = sum / 256.0;
                }
            }
            blend(&map, f)
        }
    };
    levels / bits.top() as f32
}

fn texels(words: &[u32], width: u32, log_bits: u32) -> ShadowTexels<'_> {
    ShadowTexels {
        words,
        width,
        origin: UVec2::ZERO,
        log_bits,
    }
}

#[test]
fn row_reads_match_texel_reads_everywhere() {
    let mut rng = EffectRng::new(7, 11);
    for (log_bits, width) in (0..4).flat_map(|b| [5u32, 7, 13, 31, 32, 33, 40].map(|w| (b, w))) {
        let height = 9u32;
        let words: Vec<u32> = (0..((width * height) << log_bits).div_ceil(32) + 1)
            .map(|_| rng.next_u32() & rng.next_u32())
            .collect();
        let bits = ShadowTexels {
            origin: UVec2::new(3, 5),
            ..texels(&words, width, log_bits)
        };
        // Every footprint over the triangle's texels and the padding around them.
        for i in 0..2000 {
            let pixel = Vec2::new(
                3.0 + (i as f32 * 0.618).fract() * width as f32,
                5.0 + (i as f32 * 0.414).fract() * height as f32,
            );
            for quality in 0..4 {
                let rows = bits.shadow(pixel, quality);
                let texels = texel_by_texel(&bits, pixel, quality);
                assert_eq!(rows, texels, "{log_bits} {width} {pixel} {quality}");
                assert!((0.0..=1.0).contains(&rows), "{rows}");
            }
        }
    }
}

#[test]
fn reads_above_and_before_the_grid_repeat_its_first_texels() {
    let words = [0b1011_0001u32, 0];
    let bits = texels(&words, 8, 0);
    assert_eq!(bits.row(IVec2::new(0, -2)), bits.row(IVec2::ZERO));
    // Two texels before the first: copies of texel 0, then texels 0 to 5.
    assert_eq!(bits.row(IVec2::new(-2, 0)), 0b1100_0111);
    // A run past the right side continues in the next row, as Henry's index does.
    assert_eq!(bits.row(IVec2::new(6, 0)) & 0b11, 0b10);
    // Four-bit levels: texel k is nibble k.
    let words = [0x7654_3215u32, 0xfedc_ba98];
    let levels = texels(&words, 16, 2);
    assert_eq!(levels.row(IVec2::ZERO), 0x7654_3215);
    assert_eq!(levels.row(IVec2::new(-2, 0)), 0x5432_1555);
    assert_eq!(levels.row(IVec2::new(-20, 0)), 0x5555_5555);
    assert_eq!(levels.row(IVec2::new(0, -3)), 0x7654_3215);
}

#[test]
fn runs_across_a_word_boundary_join_both_words() {
    let words = [0xf000_0000u32, 0x0000_0005];
    let bits = texels(&words, 64, 0);
    assert_eq!(bits.row(IVec2::new(28, 0)), 0b0101_1111);
    assert_eq!(bits.row(IVec2::new(24, 0)), 0xf0);
    let words = [0x7654_3210u32, 0xfedc_ba98];
    assert_eq!(texels(&words, 16, 2).row(IVec2::new(3, 0)), 0xa987_6543);
    assert_eq!(texels(&words, 32, 1).row(IVec2::new(13, 0)), 0xa61d);
}

#[test]
fn window_bit_counts_equal_the_float_sums_for_every_block() {
    for block in 0..1u32 << 16 {
        let mut map = [[0.0f32; 4]; 4];
        for (y, line) in map.iter_mut().enumerate() {
            for (x, value) in line.iter_mut().enumerate() {
                *value = ((block >> (4 * y + x)) & 1) as f32;
            }
        }
        let middle = map[1][1] + map[1][2] + map[2][1] + map[2][2];
        let top = middle + map[0][1] + map[0][2];
        let bottom = middle + map[3][1] + map[3][2];
        let sums = [
            top + map[0][0] + map[1][0] + map[2][0],
            top + map[0][3] + map[1][3] + map[2][3],
            bottom + map[1][0] + map[2][0] + map[3][0],
            bottom + map[1][3] + map[2][3] + map[3][3],
        ];
        let counts = window_counts(block).map(|count| count as f32);
        assert_eq!(counts, sums, "block {block:#06x}");
    }
}

#[test]
fn levels_scale_to_fractions() {
    assert_eq!(
        (log_bits(1), log_bits(2), log_bits(4), log_bits(8)),
        (0, 1, 2, 3)
    );
    let words = [0x0000_00f5u32; 4];
    // Texel 0 is level 5 of 15, texel 1 fully lit; the Low tier averages its 3×3 box.
    let levels = texels(&words, 8, 2);
    assert_eq!(levels.texel(IVec2::ZERO), 5.0);
    assert_eq!(
        levels.shadow(Vec2::new(0.5, 0.5), QUALITY_INTEGRATED),
        5.0 / 15.0
    );
}

#[test]
fn eight_bit_rows_hold_four_texels_and_read_on_for_the_gaussian() {
    let words = [0x4433_2211u32, 0x8877_6655, 0xccbb_aa99];
    let levels = texels(&words, 12, 3);
    assert_eq!(levels.top(), 255);
    assert_eq!(levels.row(IVec2::ZERO), 0x4433_2211);
    assert_eq!(levels.row(IVec2::new(1, 0)), 0x5544_3322);
    assert_eq!(levels.row(IVec2::new(-2, 0)), 0x2211_1111);
    assert_eq!(levels.row(IVec2::new(-9, 0)), 0x1111_1111);
    assert_eq!(levels.run(IVec2::new(3, 0)), [0x7766_5544, 0xbbaa_9988]);
    assert_eq!(levels.run(IVec2::new(-2, 0)), [0x2211_1111, 0x6655_4433]);
}
