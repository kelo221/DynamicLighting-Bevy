//! Sun maps: per baked sun, an orthographic depth map of the static scene seen from the sun, the
//! sun's counterpart of a distance cube. Meshes without baked shadows (moving ones) read it to
//! find where static geometry shades them.
//!
//! The map's square covers every static triangle across the sun's rays, and its depth runs from
//! the plane in front of the nearest triangle to the farthest one. Every texel is one closest-hit
//! ray from that plane along the sun's rays through the texel center.

use bevy::math::{Vec3, Vec4};
use rayon::prelude::*;

use crate::light::SunDisk;

use super::format::SunMap;
use super::occluder::{Occluders, RayCaster};
use super::progress::BakeProgress;

/// Sun map side in texels unless [`super::BakeSettings::sun_map_resolution`] says otherwise.
pub const SUN_MAP_RESOLUTION: u32 = 1024;
/// How far the map's ends lie in front of and behind the static scene, as a share of its depth
/// plus a fixed margin in meters, so the rays start outside every triangle.
const DEPTH_MARGIN: (f32, f32) = (0.01, 0.05);

/// The sun map of `sun` over `occluders`, with `resolution²` texels.
pub fn bake_sun_map(
    sun: &SunDisk,
    occluders: &(impl RayCaster + ?Sized),
    vertices: impl Iterator<Item = Vec3>,
    resolution: u32,
    alpha_test: bool,
    progress: &BakeProgress,
) -> SunMap {
    let frame = SunFrame::new(sun);
    let Some(bounds) = frame.bounds(vertices) else {
        // No static geometry: one texel that nothing is behind.
        return SunMap {
            rows: [Vec4::ZERO; 3],
            resolution: 1,
            texels: vec![pack_sun_depth(1.0, 1.0)],
        };
    };
    let rows = frame.rows(&bounds);
    let texel = bounds.extent / resolution as f32;
    let rows_of_depths: Vec<Vec<f32>> = (0..resolution)
        .into_par_iter()
        .map(|y| {
            if progress.is_cancelled() {
                return Vec::new();
            }
            (0..resolution)
                .map(|x| {
                    let across = bounds.min.x + (x as f32 + 0.5) * texel;
                    let down = bounds.min.y + (y as f32 + 0.5) * texel;
                    let origin = frame.world(Vec3::new(across, down, bounds.min.z));
                    occluders
                        .closest_hit(origin, frame.along, bounds.depth, alpha_test)
                        .map_or(1.0, |hit| hit.distance / bounds.depth)
                })
                .collect()
        })
        .collect();
    let depths = rows_of_depths.concat();
    let texels = depths
        .chunks(2)
        .map(|pair| pack_sun_depth(pair[0], pair.get(1).copied().unwrap_or(1.0)))
        .collect();
    progress.advance(1);
    SunMap {
        rows,
        resolution,
        texels,
    }
}

/// Packs two map-space depths into one word as unorm16, `lower` in bits 0–15.
pub fn pack_sun_depth(lower: f32, upper: f32) -> u32 {
    let unorm = |depth: f32| (depth.clamp(0.0, 1.0) * 65535.0).round() as u32;
    unorm(upper) << 16 | unorm(lower)
}

/// The sun's orthonormal frame: two axes across its rays and the direction they travel.
struct SunFrame {
    side: Vec3,
    up: Vec3,
    along: Vec3,
}

/// The static scene in sun frame coordinates: the square it fills across the rays (`min.xy`
/// plus `extent`) and the depth range along them (`min.z` plus `depth`).
struct Bounds {
    min: Vec3,
    extent: f32,
    depth: f32,
}

impl SunFrame {
    fn new(sun: &SunDisk) -> Self {
        let toward = sun.toward();
        let (side, up) = toward.any_orthonormal_pair();
        Self {
            side,
            up,
            along: -toward,
        }
    }

    fn local(&self, world: Vec3) -> Vec3 {
        Vec3::new(
            world.dot(self.side),
            world.dot(self.up),
            world.dot(self.along),
        )
    }

    fn world(&self, local: Vec3) -> Vec3 {
        self.side * local.x + self.up * local.y + self.along * local.z
    }

    fn bounds(&self, vertices: impl Iterator<Item = Vec3>) -> Option<Bounds> {
        let (min, max) = vertices
            .map(|v| self.local(v))
            .fold((Vec3::INFINITY, Vec3::NEG_INFINITY), |(min, max), v| {
                (min.min(v), max.max(v))
            });
        if !(min.is_finite() && max.is_finite()) {
            return None;
        }
        let size = max - min;
        let margin = size.z * DEPTH_MARGIN.0 + DEPTH_MARGIN.1;
        // A square around the scene with room for one margin on every side.
        let extent = size.x.max(size.y) + 2.0 * margin;
        let center = (min + max) * 0.5;
        Some(Bounds {
            min: Vec3::new(
                center.x - extent * 0.5,
                center.y - extent * 0.5,
                min.z - margin,
            ),
            extent,
            depth: size.z + 2.0 * margin,
        })
    }

    fn rows(&self, bounds: &Bounds) -> [Vec4; 3] {
        let row = |axis: Vec3, min: f32, length: f32| (axis / length).extend(-min / length);
        [
            row(self.side, bounds.min.x, bounds.extent),
            row(self.up, bounds.min.y, bounds.extent),
            row(self.along, bounds.min.z, bounds.depth),
        ]
    }
}

/// Static triangle corners, for the map's bounds.
pub fn occluder_vertices<'a>(occluders: &'a Occluders) -> impl Iterator<Item = Vec3> + 'a {
    occluders
        .triangles
        .iter()
        .flat_map(|triangle| triangle.vertices.map(Vec3::from))
}

#[cfg(test)]
mod tests {
    use bevy::math::Quat;

    use super::*;
    use crate::bake::input::BakeMesh;
    use crate::bake::tests::{cuboid, floor};

    /// The stored depth and the depth of `world` itself, in map space.
    fn depths(map: &SunMap, world: Vec3) -> (f32, f32) {
        let m = map.rows.map(|row| row.truncate().dot(world) + row.w);
        let size = map.resolution as f32;
        let texel = (Vec3::new(m[0], m[1], 0.0) * size).floor();
        let index = (texel.y * size + texel.x) as usize;
        let word = map.texels[index / 2] >> (16 * (index % 2) as u32);
        ((word & 0xFFFF) as f32 / 65535.0, m[2])
    }

    fn bake(sun: &SunDisk, meshes: &[BakeMesh], resolution: u32) -> SunMap {
        let meshes: Vec<&BakeMesh> = meshes.iter().collect();
        let occluders = Occluders::build(&meshes);
        let vertices = occluder_vertices(&occluders);
        bake_sun_map(
            sun,
            &occluders,
            vertices,
            resolution,
            false,
            &BakeProgress::new(),
        )
    }

    #[test]
    fn maps_store_the_first_surface_along_the_rays() {
        let scene = [
            floor(1, Vec3::ZERO, 5.0),
            cuboid(2, Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 2.0, 1.0)),
        ];
        let overhead = SunDisk {
            rotation: Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2),
            angular_diameter: 0.53,
        };
        let map = bake(&overhead, &scene, 64);
        assert_eq!((map.resolution, map.texels.len()), (64, 64 * 64 / 2));
        // Over the box the first surface is its top; beside it the floor.
        for (receiver, shaded) in [
            (Vec3::new(0.2, 1.0, -0.3), true),
            (Vec3::new(0.2, 2.5, -0.3), false),
            (Vec3::new(3.0, 0.5, 3.0), false),
            (Vec3::new(3.0, -0.5, 3.0), true),
        ] {
            let (stored, depth) = depths(&map, receiver);
            assert_eq!(depth > stored, shaded, "{receiver}: {depth} vs {stored}");
        }
        let (top, _) = depths(&map, Vec3::new(0.0, 0.0, 0.0));
        let (ground, _) = depths(&map, Vec3::new(4.0, 0.0, -4.0));
        let depth_per_meter = map.rows[2].truncate().length();
        assert!(
            ((ground - top) / depth_per_meter - 2.0).abs() < 0.01,
            "box height"
        );
    }

    #[test]
    fn maps_cover_the_scene_across_slanted_rays() {
        let scene = [floor(1, Vec3::new(10.0, 0.0, 0.0), 2.0)];
        let low = SunDisk {
            rotation: Quat::from_rotation_arc(Vec3::Z, Vec3::new(1.0, 1.0, 0.5).normalize()),
            angular_diameter: 0.0,
        };
        let map = bake(&low, &scene, 32);
        for corner in [(8.0, -2.0), (12.0, 2.0), (8.0, 2.0), (12.0, -2.0)] {
            let world = Vec3::new(corner.0, 0.0, corner.1);
            let m = map.rows.map(|row| row.truncate().dot(world) + row.w);
            assert!(m.iter().all(|c| (0.0..=1.0).contains(c)), "{world}: {m:?}");
            let (stored, depth) = depths(&map, world + Vec3::Y * 0.5);
            assert!(depth < stored, "above the floor is lit");
        }
        let (stored, depth) = depths(&map, Vec3::new(10.0, -0.5, 0.0));
        assert!(depth > stored, "below the floor is shaded");
    }

    #[test]
    fn empty_scenes_give_a_map_nothing_is_behind() {
        let sun = SunDisk {
            rotation: Quat::from_rotation_x(-1.0),
            angular_diameter: 0.53,
        };
        let map = bake(&sun, &[], 256);
        assert_eq!(map.resolution, 1);
        let (stored, depth) = depths(&map, Vec3::new(3.0, -40.0, 2.0));
        assert!(depth <= stored);
    }
}
