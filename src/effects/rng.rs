//! Deterministic random numbers for effects: one PCG32 stream per light, so results don't depend
//! on the order lights are processed in.

/// SplitMix64 finalizer: a stateless 64-bit mix with good avalanche.
#[inline]
pub fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// PCG32 (XSH-RR): small, fast and statistically solid for visual randomness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectRng {
    state: u64,
    increment: u64,
}

impl EffectRng {
    const MULTIPLIER: u64 = 6_364_136_223_846_793_005;

    /// A stream for one light: `seed` is the scene seed, `light_key` the light's stable id.
    pub fn for_light(seed: u64, light_key: u64) -> Self {
        Self::new(splitmix64(seed ^ splitmix64(light_key)), light_key)
    }

    pub fn new(seed: u64, stream: u64) -> Self {
        let mut rng = Self {
            state: 0,
            increment: (stream << 1) | 1,
        };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(seed);
        rng.next_u32();
        rng
    }

    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(self.increment);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        xorshifted.rotate_right((old >> 59) as u32)
    }

    /// Uniform in `[0, 1)` with 24 bits of precision.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }

    /// Uniform integer in `[0, bound)`, without modulo bias. `bound` must be non-zero.
    pub fn below(&mut self, bound: u32) -> u32 {
        debug_assert!(bound > 0);
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next_u32();
            if value >= threshold {
                return value % bound;
            }
        }
    }
}

impl Default for EffectRng {
    fn default() -> Self {
        Self::for_light(0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_repeat_per_seed_and_differ_between_lights() {
        let mut a = EffectRng::for_light(7, 1);
        let mut b = EffectRng::for_light(7, 1);
        let mut c = EffectRng::for_light(7, 2);
        let a: Vec<u32> = (0..16).map(|_| a.next_u32()).collect();
        let b: Vec<u32> = (0..16).map(|_| b.next_u32()).collect();
        let c: Vec<u32> = (0..16).map(|_| c.next_u32()).collect();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn floats_are_uniform_in_unit_interval() {
        let mut rng = EffectRng::for_light(1, 99);
        let mut buckets = [0u32; 10];
        let samples = 100_000;
        for _ in 0..samples {
            let value = rng.next_f32();
            assert!((0.0..1.0).contains(&value));
            buckets[(value * 10.0) as usize] += 1;
        }
        for count in buckets {
            let share = count as f32 / samples as f32;
            assert!((share - 0.1).abs() < 0.01, "bucket share {share}");
        }
    }

    #[test]
    fn below_covers_the_range_evenly() {
        let mut rng = EffectRng::for_light(3, 4);
        let mut counts = [0u32; 3];
        for _ in 0..30_000 {
            counts[rng.below(3) as usize] += 1;
        }
        for count in counts {
            assert!((9_000..11_000).contains(&count), "{counts:?}");
        }
    }

    #[test]
    fn splitmix_spreads_neighbouring_inputs() {
        let differing_bits = (splitmix64(1) ^ splitmix64(2)).count_ones();
        assert!((16..=48).contains(&differing_bits));
    }
}
