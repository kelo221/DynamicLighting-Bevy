//! CPU mirror of `sky_probe_openness` in `shaders/sky_probes.wesl`: how much of a baked sky a
//! point on a mesh without bake data sees.

use bevy::math::{UVec3, Vec3};

use crate::bake::SkyProbes;

/// The probes' openness at `world` for unit `normal`, 0 to 1: trilinear over the grid (clamped
/// to it), then the ambient cube's squared-normal weights.
pub fn sky_probe_openness(probes: &SkyProbes, world: Vec3, normal: Vec3) -> f32 {
    let last = probes.counts - 1;
    let grid = ((world - probes.origin) / probes.spacing).clamp(Vec3::ZERO, last.as_vec3());
    let base = grid.as_uvec3().min(probes.counts.max(UVec3::splat(2)) - 2);
    let f = grid - base.as_vec3();
    let mut cube = [0.0; 6];
    for corner in 0..8u32 {
        let step = UVec3::new(corner & 1, (corner >> 1) & 1, corner >> 2);
        let w = Vec3::select(step.cmpeq(UVec3::ONE), f, 1.0 - f);
        let weight = w.x * w.y * w.z;
        let openness = probes.openness((base + step).min(last));
        cube.iter_mut()
            .zip(openness)
            .for_each(|(sum, value)| *sum += weight * value);
    }
    let along = |axis: usize, component: f32| cube[2 * axis + usize::from(component <= 0.0)];
    normal.x * normal.x * along(0, normal.x)
        + normal.y * normal.y * along(1, normal.y)
        + normal.z * normal.z * along(2, normal.z)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 × 2 × 2 grid with spacing 2 from the origin; probe `i` has the value `values[i]` on
    /// every axis but −Y, which has half of it.
    fn grid(values: [f32; 8]) -> SkyProbes {
        SkyProbes {
            origin: Vec3::ZERO,
            spacing: 2.0,
            counts: UVec3::splat(2),
            words: values
                .into_iter()
                .flat_map(|v| SkyProbes::pack([v, v, v, v * 0.5, v, v]))
                .collect(),
        }
    }

    #[test]
    fn probes_blend_trilinearly_and_clamp_to_the_grid() {
        let values = [0.0, 1.0, 0.2, 0.4, 0.6, 0.8, 1.0, 0.0];
        let probes = grid(values);
        let at = |x: f32, y: f32, z: f32| sky_probe_openness(&probes, Vec3::new(x, y, z), Vec3::X);
        let byte = |v: f32| (v * 255.0).round() / 255.0;
        assert!((at(0.0, 0.0, 0.0) - byte(0.0)).abs() < 1e-6);
        assert!((at(2.0, 0.0, 0.0) - byte(1.0)).abs() < 1e-6);
        assert!((at(1.0, 0.0, 0.0) - 0.5).abs() < 1e-6, "halfway along x");
        let center = values.map(byte).iter().sum::<f32>() / 8.0;
        assert!((at(1.0, 1.0, 1.0) - center).abs() < 1e-6);
        assert_eq!(at(-5.0, -1.0, -3.0), at(0.0, 0.0, 0.0), "clamped below");
        assert_eq!(at(9.0, 0.0, 0.0), at(2.0, 0.0, 0.0), "clamped above");
    }

    #[test]
    fn normals_weight_the_ambient_cube() {
        let probes = grid([0.8; 8]);
        let open = |normal: Vec3| sky_probe_openness(&probes, Vec3::ONE, normal.normalize());
        let (up, down) = (open(Vec3::Y), open(Vec3::NEG_Y));
        assert!(
            (up - 0.8).abs() < 0.003 && (down - 0.4).abs() < 0.003,
            "{up} {down}"
        );
        // Halfway between up and sideways, half of each.
        let tilted = open(Vec3::new(1.0, 1.0, 0.0));
        assert!((tilted - 0.5 * (up + open(Vec3::X))).abs() < 1e-6);
        // The squared components of a unit normal add up to 1, so axes that agree give their
        // value for any normal facing them.
        let value = open(Vec3::new(0.3, 0.5, -0.8));
        assert!((value - up).abs() < 1e-6, "{value}");
    }

    #[test]
    fn single_probe_grids_read_their_one_probe() {
        let probes = SkyProbes {
            origin: Vec3::new(4.0, 0.0, 0.0),
            spacing: 1.0,
            counts: UVec3::ONE,
            words: SkyProbes::pack([1.0, 0.0, 1.0, 0.0, 1.0, 0.0]).to_vec(),
        };
        assert_eq!(sky_probe_openness(&probes, Vec3::splat(-9.0), Vec3::Y), 1.0);
        assert_eq!(sky_probe_openness(&probes, Vec3::ZERO, Vec3::NEG_Z), 0.0);
    }
}
