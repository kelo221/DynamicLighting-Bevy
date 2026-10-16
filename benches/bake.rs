//! Bake benchmark: a synthetic room (floor, crates, lights on a grid) baked without bounce, with
//! bounce, and with 4× the lights. Prints stable `dynlight bench:` lines with the time, texels
//! and rays per second.
//!
//! `cargo bench --bench bake`. Times are the best of three runs on all cores.

use std::time::{Duration, Instant};

use bevy::math::{Vec2, Vec3};
use dynlight::bake::{
    BakeBounce, BakeInput, BakeLight, BakeMesh, BakeOptions, BakeProgress, BakedScene, bake,
};
use dynlight::light::{Illumination, Transparency};
use dynlight::shared::{BakeId, DynamicGeometryMode, mesh_content_hash};

const ROOM: f32 = 24.0;
const RUNS: usize = 3;

/// Positions, normals, UV1 and indices of part of a mesh.
type Part = (Vec<Vec3>, Vec<Vec3>, Vec<Vec2>, Vec<u32>);

/// One quad as two triangles, with UV1 covering the whole square.
fn quad(corners: [Vec3; 4], normal: Vec3) -> Part {
    let uv = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(u, v)| Vec2::new(u, v));
    (
        corners.to_vec(),
        vec![normal; 4],
        uv.to_vec(),
        vec![0, 2, 1, 0, 3, 2],
    )
}

fn mesh(id: u64, parts: Vec<Part>) -> BakeMesh {
    let (mut positions, mut normals, mut uv1, mut indices) = (vec![], vec![], vec![], vec![]);
    // Each face gets its own cell of a UV1 atlas so bounce maps don't overlap.
    let cells = (parts.len() as f32).sqrt().ceil();
    for (k, (p, n, uv, i)) in parts.into_iter().enumerate() {
        let base = positions.len() as u32;
        let cell = Vec2::new((k as f32 % cells).floor(), (k as f32 / cells).floor());
        positions.extend(p);
        normals.extend(n);
        uv1.extend(uv.iter().map(|uv| (cell + *uv * 0.9 + 0.05) / cells));
        indices.extend(i.iter().map(|i| base + i));
    }
    let raw = |v: &[Vec3]| v.iter().map(|v| v.to_array()).collect::<Vec<_>>();
    let raw_uv: Vec<[f32; 2]> = uv1.iter().map(|uv| uv.to_array()).collect();
    BakeMesh {
        id: BakeId(id),
        content_hash: mesh_content_hash(&raw(&positions), &indices, &raw_uv),
        positions,
        normals,
        indices,
        uv1,
        ..Default::default()
    }
}

fn floor() -> BakeMesh {
    let h = ROOM / 2.0;
    let corners = [
        Vec3::new(-h, 0.0, -h),
        Vec3::new(h, 0.0, -h),
        Vec3::new(h, 0.0, h),
        Vec3::new(-h, 0.0, h),
    ];
    mesh(1, vec![quad(corners, Vec3::Y)])
}

/// An axis-aligned box with its five visible faces.
fn crate_at(id: u64, center: Vec3, half: f32) -> BakeMesh {
    let faces = [Vec3::Y, Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z];
    let parts = faces
        .iter()
        .map(|&normal| {
            let (u, v) = normal.any_orthonormal_pair();
            let c = center + normal * half;
            let corners = [
                c - u * half - v * half,
                c + u * half - v * half,
                c + u * half + v * half,
                c - u * half + v * half,
            ];
            // Wind the face towards its normal.
            if (corners[1] - corners[0])
                .cross(corners[2] - corners[0])
                .dot(normal)
                < 0.0
            {
                quad([corners[0], corners[3], corners[2], corners[1]], normal)
            } else {
                quad(corners, normal)
            }
        })
        .collect();
    mesh(id, parts)
}

fn scene(lights_per_side: u32, bounce: bool) -> BakeInput {
    let mut meshes = vec![floor()];
    for i in 0..36u32 {
        let (x, z) = ((i % 6) as f32, (i / 6) as f32);
        let center = Vec3::new((x - 2.5) * 3.6, 0.5, (z - 2.5) * 3.6);
        meshes.push(crate_at(100 + u64::from(i), center, 0.5));
    }
    let spacing = ROOM / lights_per_side as f32;
    let lights = (0..lights_per_side * lights_per_side)
        .map(|i| {
            let (x, z) = ((i % lights_per_side) as f32, (i / lights_per_side) as f32);
            BakeLight {
                id: BakeId(10_000 + u64::from(i)),
                position: Vec3::new(
                    (x + 0.5) * spacing - ROOM / 2.0,
                    2.0,
                    (z + 0.5) * spacing - ROOM / 2.0,
                ),
                radius: 5.0,
                illumination: if bounce {
                    Illumination::SingleBounce
                } else {
                    Illumination::Direct
                },
                transparency: Transparency::Disabled,
                bounce: BakeBounce::default(),
                area: None,
            }
        })
        .collect();
    BakeInput { meshes, lights }
}

fn best_of(input: &BakeInput, options: &BakeOptions) -> (Duration, BakedScene) {
    let mut best = None::<(Duration, BakedScene)>;
    for _ in 0..RUNS {
        let started = Instant::now();
        let scene = bake(input, options, &BakeProgress::new()).expect("the bench scene bakes");
        let elapsed = started.elapsed();
        if best.as_ref().is_none_or(|(time, _)| elapsed < *time) {
            best = Some((elapsed, scene));
        }
    }
    best.expect("at least one run")
}

fn main() {
    let options = BakeOptions {
        pixel_density: 16.0,
        dynamic_geometry: DynamicGeometryMode::Angular,
        ..Default::default()
    };
    for (name, input) in [
        ("direct, 64 lights", scene(8, false)),
        ("bounce, 64 lights", scene(8, true)),
        ("direct, 256 lights", scene(16, false)),
    ] {
        let (time, scene) = best_of(&input, &options);
        let seconds = time.as_secs_f64();
        let stats = &scene.stats;
        let rays = stats.rays + stats.bounce_rays;
        println!(
            "dynlight bench: {name}: ms {}, meshes {}, texels {}, rays {}, rays/s {:.1}M",
            time.as_millis(),
            stats.meshes,
            stats.texels,
            rays,
            rays as f64 / seconds / 1e6
        );
    }
}
