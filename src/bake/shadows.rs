//! Henry's `OptimizeTriangle` and `BuildShadows`: drop lights that never reach a triangle, then
//! cut each remaining (triangle, light) pair's shadows out of the mesh-wide map with 2 px of
//! dilated padding.

use rayon::prelude::*;

use super::bits::TexelRect;
use super::bounce_pack::pack_bounce;
use super::mesh_bake::{LightRegions, Scene, TriangleSetup};
use super::mesh_builder::{PreparedMesh, optimize_rect};
use super::random;
use super::seams::dilate_shadows;
use super::shadow_grid::{ShadowGrid, ShadowRegion};
use super::triangle_words::{TriangleLight, TriangleRecord};

/// One light of one triangle.
struct Entry {
    light: u32,
    /// Bounce values over the padded crop grid, when the light bounces onto the triangle.
    bounce: Option<Vec<f32>>,
}

/// Every triangle's light entries, from association to serialization.
pub struct LightEntries {
    triangles: Vec<(TexelRect, Vec<Entry>)>,
}

impl LightEntries {
    /// Pairs each triangle's lights with its bounce textures (`bounce[t][k]` belongs to light
    /// `k` of triangle `t`; an empty `bounce` means no bounce in this mesh).
    pub fn new(setups: Vec<TriangleSetup>, bounce: &mut [Vec<Option<Vec<f32>>>]) -> Self {
        let triangles = setups
            .into_iter()
            .enumerate()
            .map(|(t, setup)| {
                let lights = setup
                    .lights
                    .into_iter()
                    .enumerate()
                    .map(|(k, light)| Entry {
                        light,
                        bounce: bounce.get_mut(t).and_then(|b| b.get_mut(k)?.take()),
                    })
                    .collect();
                (setup.crop, lights)
            })
            .collect();
        Self { triangles }
    }

    /// Removes lights without a single lit texel near the triangle (the triangle's texels grown
    /// by one), unless they bounce light onto it. Returns how many were removed.
    pub fn optimize(
        &mut self,
        prepared: &PreparedMesh,
        regions: &LightRegions,
        bits: &[ShadowRegion],
    ) -> u64 {
        self.triangles
            .par_iter_mut()
            .enumerate()
            .map(|(triangle, (_, lights))| {
                let rect = optimize_rect(prepared.uvs(triangle), prepared.resolution);
                let before = lights.len();
                lights.retain(|entry| {
                    entry.bounce.is_some()
                        || any_light(&bits[regions.slot[entry.light as usize] as usize], rect)
                });
                (before - lights.len()) as u64
            })
            .sum()
    }

    /// Which light slots any triangle still uses.
    pub fn used_lights(&self, light_count: usize) -> Vec<bool> {
        let mut used = vec![false; light_count];
        for (_, lights) in &self.triangles {
            for entry in lights {
                used[entry.light as usize] = true;
            }
        }
        used
    }

    /// Builds the serializable records: dilated shadows and packed bounce per entry.
    pub fn into_records(
        self,
        scene: &Scene,
        prepared: &PreparedMesh,
        regions: &LightRegions,
        bits: &[ShadowRegion],
    ) -> Vec<TriangleRecord> {
        self.triangles
            .into_par_iter()
            .enumerate()
            .map(|(triangle, (crop, lights))| {
                let lights = lights
                    .into_iter()
                    .map(|entry| {
                        let region = &bits[regions.slot[entry.light as usize] as usize];
                        let light = scene.lights[entry.light as usize];
                        let bounce = entry.bounce.map(|values| {
                            let compression = light
                                .bounce
                                .compression
                                .unwrap_or(scene.options.bounce_compression);
                            let mut rng = random::stream(&[
                                scene.options.seed,
                                prepared.mesh.id.0,
                                light.id.0,
                                triangle as u64,
                            ]);
                            pack_bounce(&values, compression, &mut rng)
                        });
                        TriangleLight {
                            light: entry.light,
                            shadow: Some(crop_shadows(region, crop).into_words()),
                            bounce,
                        }
                    })
                    .collect();
                TriangleRecord { crop, lights }
            })
            .collect()
    }
}

/// Whether any texel of `region` inside `rect` gets some light.
fn any_light(region: &ShadowRegion, rect: TexelRect) -> bool {
    let r = region.rect;
    let (min_x, max_x) = (rect.min_x.max(r.min_x), rect.max_x.min(r.max_x));
    let (min_y, max_y) = (rect.min_y.max(r.min_y), rect.max_y.min(r.max_y));
    if min_x > max_x || min_y > max_y {
        return false;
    }
    (min_y..=max_y).any(|y| (min_x..=max_x).any(|x| region.grid.get(x - r.min_x, y - r.min_y) != 0))
}

/// The shadows of `crop` copied into a grid with 2 px of padding on every side, then dilated
/// into the padding.
pub fn crop_shadows(region: &ShadowRegion, crop: TexelRect) -> ShadowGrid {
    let bits = region.grid.bits_per_texel();
    let mut grid = ShadowGrid::new(crop.width() + 4, crop.height() + 4, bits);
    for y in crop.min_y..=crop.max_y {
        for x in crop.min_x..=crop.max_x {
            let level = region.get(i64::from(x), i64::from(y));
            grid.set(x - crop.min_x + 2, y - crop.min_y + 2, level);
        }
    }
    dilate_shadows(&mut grid);
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(min_x: u32, min_y: u32, max_x: u32, max_y: u32) -> TexelRect {
        TexelRect {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    #[test]
    fn crops_offset_by_two_and_dilate() {
        for bits in [1, 4] {
            let mut grid = ShadowGrid::new(10, 10, bits);
            grid.set(4, 5, 1);
            let region = ShadowRegion {
                rect: rect(0, 0, 9, 9),
                grid,
            };
            let crop = rect(4, 5, 6, 6);
            let cropped = crop_shadows(&region, crop);
            assert_eq!(cropped.bits_per_texel(), bits);
            assert_eq!((cropped.width(), cropped.height()), (7, 6));
            assert_eq!(cropped.get(2, 2), 1, "texel (4, 5) lands at (2, 2)");
            assert_eq!((cropped.get(0, 0), cropped.get(1, 1)), (1, 1), "dilated");
            assert_eq!(cropped.get(3, 3), 0, "interior texels are not dilated");
        }
    }

    #[test]
    fn any_light_only_looks_inside_both_rects() {
        let mut grid = ShadowGrid::new(4, 4, 4);
        grid.set(3, 3, 2);
        let region = ShadowRegion {
            rect: rect(10, 10, 13, 13),
            grid,
        };
        assert!(any_light(&region, rect(12, 12, 20, 20)));
        assert!(!any_light(&region, rect(0, 0, 12, 12)));
        assert!(!any_light(&region, rect(0, 0, 5, 5)));
    }
}
