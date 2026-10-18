//! Sky probes: the baked skies' shadows for meshes without bake data. A grid over the static
//! scene where every probe traces, per axis, the rays of a texel facing that axis, and stores how
//! much of the sky each of the six hemispheres sees.
//!
//! Probes inside geometry would darken everything near them, so a probe whose rays mostly reach
//! the back of a triangle takes the average of its valid neighbours instead, layer by layer.

use bevy::math::{UVec3, Vec3};
use rayon::prelude::*;

use super::format::SkyProbes;
use super::input::BakeError;
use super::occluder::{Occluders, RayCaster};
use super::progress::BakeProgress;
use super::random;
use super::sky::sky_visibility;

/// The most probes a grid may have; scenes that would need more space them further apart.
pub const MAX_SKY_PROBES: usize = 65_536;
/// A probe is inside geometry when more than this share of its rays reach a triangle's back.
const INSIDE_SHARE: f32 = 0.25;
/// Keys the probes' random streams apart from the texels'.
const PROBE_STREAM: u64 = 0x736b_7970_726f_6265;
/// +X, −X, +Y, −Y, +Z and −Z.
const AXES: [Vec3; 6] = [
    Vec3::X,
    Vec3::NEG_X,
    Vec3::Y,
    Vec3::NEG_Y,
    Vec3::Z,
    Vec3::NEG_Z,
];

/// Where the probes go: a grid centered on the static bounds, reaching half a spacing past them
/// so its outer probes lie outside the scene's surfaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProbeGrid {
    pub origin: Vec3,
    pub spacing: f32,
    pub counts: UVec3,
}

impl ProbeGrid {
    /// The grid over the box from `min` to `max` at `spacing` meters, widened until it has at
    /// most [`MAX_SKY_PROBES`] probes.
    pub fn over(min: Vec3, max: Vec3, spacing: f32) -> Self {
        let extent = (max - min).max(Vec3::ZERO) + spacing;
        let counts_at = |spacing: f32| (extent / spacing).ceil().as_uvec3() + 1;
        let mut spacing = spacing;
        let mut counts = counts_at(spacing);
        while SkyProbes::probe_count(counts) > MAX_SKY_PROBES {
            let ratio = SkyProbes::probe_count(counts) as f32 / MAX_SKY_PROBES as f32;
            spacing *= ratio.cbrt().max(1.01);
            counts = counts_at(spacing);
        }
        let span = (counts - 1).as_vec3() * spacing;
        Self {
            origin: (min + max) * 0.5 - span * 0.5,
            spacing,
            counts,
        }
    }

    pub fn position(&self, probe: UVec3) -> Vec3 {
        self.origin + probe.as_vec3() * self.spacing
    }

    fn probe(&self, index: usize) -> UVec3 {
        let x = self.counts.x as usize;
        let xy = x * self.counts.y as usize;
        UVec3::new(
            (index % x) as u32,
            (index % xy / x) as u32,
            (index / xy) as u32,
        )
    }
}

/// One probe's ambient cube, or `None` when it lies inside geometry.
fn trace_probe(
    occluders: &Occluders,
    position: Vec3,
    samples: u32,
    alpha_test: bool,
    key: [u64; 4],
) -> Option<[f32; 6]> {
    let mut back = 0u32;
    let mut openness = [0.0; 6];
    for (axis, (open, normal)) in openness.iter_mut().zip(AXES).enumerate() {
        let mut rng = random::stream(&[key[0], PROBE_STREAM, key[1], key[2], key[3], axis as u64]);
        let visibility = sky_visibility(normal, samples, &mut rng, |direction| {
            let hit = occluders.closest_hit(position, direction, f32::INFINITY, alpha_test);
            let hit = hit.filter(|hit| hit.distance > 0.0);
            if let Some(hit) = &hit
                && occluders.face_normal(hit).dot(direction) > 0.0
            {
                back += 1;
            }
            hit.is_some()
        });
        *open = visibility.fraction;
    }
    let rays = (samples.max(1) * AXES.len() as u32) as f32;
    (back as f32 <= INSIDE_SHARE * rays).then_some(openness)
}

/// Fills probes without a value from their valid face neighbours, one layer per pass, reading
/// only the previous pass so the result doesn't depend on the visiting order. Probes no valid
/// probe reaches are open.
fn fill_inside(grid: &ProbeGrid, probes: &mut [Option<[f32; 6]>]) {
    let counts = grid.counts.as_ivec3();
    loop {
        let previous = probes.to_vec();
        let mut changed = false;
        for (index, probe) in probes.iter_mut().enumerate() {
            if probe.is_some() {
                continue;
            }
            let at = grid.probe(index).as_ivec3();
            let mut sum = [0.0; 6];
            let mut found = 0.0;
            for step in AXES.map(|axis| axis.as_ivec3()) {
                let next = at + step;
                if next.cmplt(bevy::math::IVec3::ZERO).any() || next.cmpge(counts).any() {
                    continue;
                }
                let index = ((next.z * counts.y + next.y) * counts.x + next.x) as usize;
                if let Some(values) = previous[index] {
                    sum.iter_mut().zip(values).for_each(|(s, v)| *s += v);
                    found += 1.0;
                }
            }
            if found > 0.0 {
                *probe = Some(sum.map(|s| s / found));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// The sky probes over every static triangle, `samples` rays per probe and axis, or `None`
/// without static geometry.
pub fn bake_sky_probes(
    occluders: &Occluders,
    spacing: f32,
    samples: u32,
    alpha_test: bool,
    seed: u64,
    progress: &BakeProgress,
) -> Result<Option<SkyProbes>, BakeError> {
    let mut vertices = occluders
        .triangles
        .iter()
        .flat_map(|t| t.vertices.map(Vec3::from));
    let Some(first) = vertices.next() else {
        return Ok(None);
    };
    let (min, max) = vertices.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v)));
    let grid = ProbeGrid::over(min, max, spacing);
    let mut probes: Vec<Option<[f32; 6]>> = (0..SkyProbes::probe_count(grid.counts))
        .into_par_iter()
        .map(|index| {
            if progress.is_cancelled() {
                return None;
            }
            let probe = grid.probe(index);
            let key = [
                seed,
                u64::from(probe.x),
                u64::from(probe.y),
                u64::from(probe.z),
            ];
            trace_probe(occluders, grid.position(probe), samples, alpha_test, key)
        })
        .collect();
    progress.check()?;
    fill_inside(&grid, &mut probes);
    progress.advance(1);
    Ok(Some(SkyProbes {
        origin: grid.origin,
        spacing: grid.spacing,
        counts: grid.counts,
        words: probes
            .into_iter()
            .flat_map(|probe| SkyProbes::pack(probe.unwrap_or([1.0; 6])))
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_cover_the_bounds_and_stay_within_the_cap() {
        let grid = ProbeGrid::over(Vec3::ZERO, Vec3::new(4.0, 2.0, 0.0), 1.0);
        assert_eq!(grid.counts, UVec3::new(6, 4, 2));
        assert_eq!(grid.position(UVec3::ZERO), Vec3::new(-0.5, -0.5, -0.5));
        assert_eq!(grid.position(grid.counts - 1), Vec3::new(4.5, 2.5, 0.5));
        let wide = ProbeGrid::over(Vec3::ZERO, Vec3::new(500.0, 20.0, 300.0), 1.0);
        assert!(SkyProbes::probe_count(wide.counts) <= MAX_SKY_PROBES);
        assert!(wide.spacing > 1.0);
        let reach = wide.position(wide.counts - 1) - wide.position(UVec3::ZERO);
        assert!(reach.cmpge(Vec3::new(500.0, 20.0, 300.0)).all(), "{reach}");
        for index in [0, 7, 100, SkyProbes::probe_count(wide.counts) - 1] {
            let probe = wide.probe(index);
            let back = (probe.z * wide.counts.y + probe.y) * wide.counts.x + probe.x;
            assert_eq!(back as usize, index);
        }
    }

    #[test]
    fn inside_probes_take_their_neighbours_values() {
        let grid = ProbeGrid {
            origin: Vec3::ZERO,
            spacing: 1.0,
            counts: UVec3::new(4, 1, 1),
        };
        let mut probes = vec![Some([0.5; 6]), None, None, Some([1.0; 6])];
        fill_inside(&grid, &mut probes);
        assert_eq!(probes[1], Some([0.5; 6]));
        assert_eq!(probes[2], Some([1.0; 6]));
        let mut lost = vec![None; 4];
        fill_inside(&grid, &mut lost);
        assert!(lost.iter().all(Option::is_none), "left open by the packing");
    }
}
