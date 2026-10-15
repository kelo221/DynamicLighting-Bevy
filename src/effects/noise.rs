//! Smooth 2D gradient noise for the flicker-style effects.

use core::f32::consts::{FRAC_1_PI, FRAC_1_SQRT_2};

use super::rng::splitmix64;

/// Domain rotation applied before sampling. Effects sample along lines such as `(t, 0)`; without
/// the rotation those would run along lattice rows, where gradient noise loses most of its range.
const ROTATION_COS: f32 = 0.877_582_6;
const ROTATION_SIN: f32 = 0.479_425_55;
const DOMAIN_OFFSET: (f32, f32) = (FRAC_1_PI, 0.618_034);

const GRADIENTS: [(f32, f32); 8] = [
    (1.0, 0.0),
    (-1.0, 0.0),
    (0.0, 1.0),
    (0.0, -1.0),
    (FRAC_1_SQRT_2, FRAC_1_SQRT_2),
    (-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
    (FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
    (-FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
];

/// Smooth noise in `[0, 1]`, centered on 0.5, varying on a unit-sized lattice.
pub fn noise01(x: f32, y: f32) -> f32 {
    let rotated_x = x * ROTATION_COS - y * ROTATION_SIN + DOMAIN_OFFSET.0;
    let rotated_y = x * ROTATION_SIN + y * ROTATION_COS + DOMAIN_OFFSET.1;
    // Unit-gradient 2D noise peaks at ±√½, so this maps the full range onto [0, 1].
    (0.5 + gradient_noise(rotated_x, rotated_y) * FRAC_1_SQRT_2).clamp(0.0, 1.0)
}

fn gradient_noise(x: f32, y: f32) -> f32 {
    let cell_x = x.floor();
    let cell_y = y.floor();
    let fx = x - cell_x;
    let fy = y - cell_y;
    let ix = cell_x as i32;
    let iy = cell_y as i32;

    let corner = |dx: i32, dy: i32| {
        let (gx, gy) = gradient(ix.wrapping_add(dx), iy.wrapping_add(dy));
        gx * (fx - dx as f32) + gy * (fy - dy as f32)
    };
    let u = fade(fx);
    let v = fade(fy);
    let bottom = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * u;
    let top = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * u;
    bottom + (top - bottom) * v
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn gradient(ix: i32, iy: i32) -> (f32, f32) {
    let key = (u64::from(ix as u32) << 32) | u64::from(iy as u32);
    GRADIENTS[(splitmix64(key) >> 61) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::rng::EffectRng;

    /// Share of `samples` above each threshold.
    fn exceedance(samples: &[f32], thresholds: &[f32]) -> Vec<f32> {
        thresholds
            .iter()
            .map(|&t| samples.iter().filter(|&&n| n > t).count() as f32 / samples.len() as f32)
            .collect()
    }

    fn random_plane_samples() -> Vec<f32> {
        let mut rng = EffectRng::for_light(11, 0);
        (0..200_000)
            .map(|_| noise01(rng.next_f32() * 4096.0, rng.next_f32() * 4096.0))
            .collect()
    }

    #[test]
    fn output_is_in_range_and_centered() {
        let samples = random_plane_samples();
        assert!(samples.iter().all(|n| (0.0..=1.0).contains(n)));
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        assert!((mean - 0.5).abs() < 0.01, "mean {mean}");
    }

    /// The effects fire on thresholds: candle and fire gusts above 0.7, candle dips above 0.8,
    /// lightning above 0.74. Those events should be occasional, not constant and not absent.
    #[test]
    fn effect_thresholds_are_crossed_occasionally() {
        let samples = random_plane_samples();
        let shares = exceedance(&samples, &[0.5, 0.7, 0.74, 0.8]);
        // Calibrated to about 10%, 6% and 2%: gusts and flashes are noticeable but rare.
        assert!((0.48..0.52).contains(&shares[0]), "P(>0.5) {}", shares[0]);
        assert!((0.07..0.14).contains(&shares[1]), "P(>0.7) {}", shares[1]);
        assert!((0.04..0.09).contains(&shares[2]), "P(>0.74) {}", shares[2]);
        assert!((0.012..0.035).contains(&shares[3]), "P(>0.8) {}", shares[3]);
    }

    /// Effects sample along lines like `(t, 0)` and `(t * 5, 20)`; those must behave like the
    /// plane as a whole rather than collapsing onto lattice rows.
    #[test]
    fn effect_sample_lines_keep_the_full_distribution() {
        for row in [0.0, 10.0, 20.0, 0.2, 4.5, 7.8] {
            let samples: Vec<f32> = (0..100_000)
                .map(|i| noise01(i as f32 * 0.037, row))
                .collect();
            let shares = exceedance(&samples, &[0.7]);
            assert!(
                (0.07..0.14).contains(&shares[0]),
                "row {row}: P(>0.7) {}",
                shares[0]
            );
            let spread = samples
                .iter()
                .fold((1.0f32, 0.0f32), |(lo, hi), &n| (lo.min(n), hi.max(n)));
            assert!(
                spread.0 < 0.2 && spread.1 > 0.8,
                "row {row}: range {spread:?}"
            );
        }
    }

    #[test]
    fn noise_is_continuous() {
        let step = 1.0e-3;
        let mut previous = noise01(0.0, 3.0);
        for i in 1..20_000 {
            let current = noise01(i as f32 * step, 3.0);
            assert!((current - previous).abs() < 0.01, "jump at {i}");
            previous = current;
        }
    }

    #[test]
    fn noise_is_deterministic_and_handles_negative_coordinates() {
        assert_eq!(noise01(12.34, -5.6), noise01(12.34, -5.6));
        let samples: Vec<f32> = (0..1000)
            .map(|i| noise01(-(i as f32) * 0.31, -7.0))
            .collect();
        assert!(samples.iter().all(|n| (0.0..=1.0).contains(n)));
        assert!(samples.iter().any(|&n| (n - samples[0]).abs() > 0.1));
    }
}
