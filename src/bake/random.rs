//! Stateless seeding for bake randomness: every random stream is keyed by what it belongs to
//! (scene seed, mesh, light, texel, …), so results don't depend on thread scheduling.

use bevy::math::Vec3;

use crate::effects::{EffectRng, splitmix64};

/// Mixes key parts into one 64-bit seed. Order matters.
pub fn seed(parts: &[u64]) -> u64 {
    parts.iter().fold(0x0b4c_e5ee_d000_0000, |acc, &part| {
        splitmix64(acc ^ splitmix64(part))
    })
}

/// A random stream for the given key.
pub fn stream(parts: &[u64]) -> EffectRng {
    let seed = seed(parts);
    EffectRng::new(seed, seed.rotate_left(17))
}

/// A uniformly distributed unit vector (rejection-free: z uniform in [-1, 1], angle uniform).
pub fn on_unit_sphere(rng: &mut EffectRng) -> Vec3 {
    let z = rng.next_f32() * 2.0 - 1.0;
    let angle = rng.next_f32() * std::f32::consts::TAU;
    let r = (1.0 - z * z).max(0.0).sqrt();
    Vec3::new(r * angle.cos(), r * angle.sin(), z)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_depend_on_every_part_and_their_order() {
        assert_eq!(seed(&[1, 2, 3]), seed(&[1, 2, 3]));
        assert_ne!(seed(&[1, 2, 3]), seed(&[1, 2, 4]));
        assert_ne!(seed(&[1, 2, 3]), seed(&[3, 2, 1]));
        assert_ne!(seed(&[0]), seed(&[0, 0]));
    }

    #[test]
    fn sphere_points_are_unit_and_centered() {
        let mut rng = stream(&[7]);
        let mut sum = Vec3::ZERO;
        for _ in 0..20_000 {
            let p = on_unit_sphere(&mut rng);
            assert!((p.length() - 1.0).abs() < 1e-4);
            sum += p;
        }
        assert!((sum / 20_000.0).length() < 0.02);
    }
}
