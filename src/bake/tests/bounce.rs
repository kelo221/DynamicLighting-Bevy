//! Single bounce: a box on a lit floor. The box side facing away from the light gets no direct
//! light but receives light bounced off the floor; Preview bakes skip bounce entirely.

use bevy::math::{Vec2, Vec3};

use super::{cuboid, floor, point_light};
use crate::bake::input::{BakeInput, BakeMesh, BakeOptions};
use crate::bake::mirror::TriangleView;
use crate::bake::progress::BakeProgress;
use crate::bake::texels::triangle_texels;
use crate::bake::{BakedScene, bake, encode};
use crate::light::{BounceCompression, Illumination};
use crate::shared::{BakeId, DynamicGeometryMode};

const BOX_MIN: Vec3 = Vec3::new(1.0, 0.0, -0.5);
const BOX_MAX: Vec3 = Vec3::new(1.5, 1.0, 0.5);

fn input() -> BakeInput {
    let mut light = point_light(10, Vec3::new(0.0, 1.5, 0.0), 5.0);
    light.illumination = Illumination::SingleBounce;
    light.bounce.samples = 12;
    BakeInput {
        meshes: vec![floor(1, Vec3::ZERO, 3.0), cuboid(2, BOX_MIN, BOX_MAX)],
        lights: vec![light],
    }
}

fn options(skip_bounce: bool) -> BakeOptions {
    BakeOptions {
        pixel_density: 8.0,
        dynamic_geometry: DynamicGeometryMode::LightingOnly,
        photon_cube_resolution: 48,
        skip_bounce,
        bounce_compression: BounceCompression::Bits6,
        ..Default::default()
    }
}

/// Decoded bounce values over the texels of `triangle` of `mesh` (light slot 0), or `None` when
/// the triangle has no entry for the light.
fn bounce_values(baked: &BakedScene, mesh: &BakeMesh, triangle: usize) -> Option<Vec<f32>> {
    let words = &baked.meshes.iter().find(|m| m.id == mesh.id).unwrap();
    let corners = &mesh.indices[triangle * 3..][..3];
    let vertices = [0, 1, 2].map(|k| mesh.positions[corners[k] as usize]);
    let normals = [0, 1, 2].map(|k| mesh.normals[corners[k] as usize]);
    let uvs: [Vec2; 3] = [0, 1, 2].map(|k| mesh.uv1[corners[k] as usize]);
    let view = TriangleView::load(&words.triangles, triangle as u32, baked.settings.bounce);
    let entry = (0..view.light_count)
        .map(|k| view.light(k))
        .find(|e| e.light_index == 0)?;
    assert_ne!(entry.bounce_offset, 0, "the entry has bounce data");
    let face = (vertices[1] - vertices[0])
        .cross(vertices[2] - vertices[0])
        .normalize();
    let texels = triangle_texels(vertices, normals, uvs, face, words.resolution);
    assert!(!texels.is_empty());
    Some(
        texels
            .iter()
            .map(|t| view.bounce_texel(&entry, t.x, t.y, 6).powi(2))
            .collect(),
    )
}

#[test]
fn the_floor_bounces_onto_the_far_side_of_the_box() {
    let input = input();
    let baked = bake(&input, &options(false), &BakeProgress::new()).unwrap();
    assert!(baked.settings.bounce);
    assert_eq!(
        baked.lights[0].bounce_compression,
        Some(BounceCompression::Bits6)
    );
    assert!(baked.stats.bounce_rays > 0);

    // Cuboid face 0 is +X, facing away from the light at x = 0: it keeps the light only
    // because of its bounce data.
    let cube = &input.meshes[1];
    for triangle in 0..2 {
        let values = bounce_values(&baked, cube, triangle).expect("bounce keeps the light");
        let lit = values.iter().filter(|&&v| v > 0.0).count();
        assert!(lit * 2 > values.len(), "{lit} of {} texels", values.len());
        assert!(values.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }
    // The -X face looks at the light and the floor; it receives bounce too.
    let facing = bounce_values(&baked, cube, 2).expect("bounce on the lit side");
    assert!(facing.iter().any(|&v| v > 0.0));

    // The floor gets light bounced off the box near it.
    let floor = &input.meshes[0];
    let floor_values: Vec<f32> = (0..2)
        .filter_map(|t| bounce_values(&baked, floor, t))
        .flatten()
        .collect();
    assert!(floor_values.iter().any(|&v| v > 0.0));
}

#[test]
fn preview_skips_bounce() {
    let input = input();
    let baked = bake(&input, &options(true), &BakeProgress::new()).unwrap();
    assert!(!baked.settings.bounce);
    assert_eq!(baked.lights[0].bounce_compression, None);
    assert_eq!(baked.stats.bounce_rays, 0);
    let cube = baked.meshes.iter().find(|m| m.id == BakeId(2)).unwrap();
    for triangle in 0..2 {
        let view = TriangleView::load(&cube.triangles, triangle, false);
        assert_eq!(view.light_count, 0, "no direct light reaches the far side");
    }
}

#[test]
fn bounce_bakes_are_byte_identical_on_any_thread_count() {
    let input = input();
    let run = |threads| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| encode(&bake(&input, &options(false), &BakeProgress::new()).unwrap()))
    };
    let single = run(1);
    assert_eq!(single, run(3));
    assert_eq!(single, run(8));
}
