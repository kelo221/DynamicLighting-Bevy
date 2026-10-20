//! Sun bounce and colored bounce. A wall standing on a wide floor under an overhead sun gets no
//! direct light but sees the sunlit floor over almost half its hemisphere, so its bounce comes
//! close to the view factor ½. Colored bounce takes the color of the surface the light bounced
//! off; without it the bounce stays white whatever the colors, byte for byte.

use bevy::math::{Vec2, Vec3};

use super::{cuboid, floor, point_light, sun_light};
use crate::bake::input::{BakeInput, BakeMesh, BakeOptions};
use crate::bake::mirror::TriangleView;
use crate::bake::progress::BakeProgress;
use crate::bake::texels::triangle_texels;
use crate::bake::{BakedScene, bake, encode};
use crate::light::{BounceCompression, Illumination};
use crate::shared::DynamicGeometryMode;

const RED: Vec3 = Vec3::new(0.8, 0.1, 0.0);

/// A thin wall (its +X side is cuboid face 0) on a 20 m floor, lit by a sun straight overhead.
fn sun_input(floor_albedo: Vec3) -> BakeInput {
    let mut sun = sun_light(10, Vec3::Y, 0.0);
    sun.illumination = Illumination::SingleBounce;
    sun.bounce.samples = 64;
    let mut ground = floor(1, Vec3::ZERO, 10.0);
    ground.albedo = floor_albedo;
    BakeInput {
        meshes: vec![
            ground,
            cuboid(2, Vec3::new(-0.02, 0.0, -0.5), Vec3::new(0.0, 1.0, 0.5)),
        ],
        lights: vec![sun],
    }
}

fn options(colored_bounce: bool) -> BakeOptions {
    BakeOptions {
        pixel_density: 8.0,
        dynamic_geometry: DynamicGeometryMode::LightingOnly,
        photon_cube_resolution: 48,
        bounce_compression: BounceCompression::Bits8,
        colored_bounce,
        ..Default::default()
    }
}

/// Decoded bounce over the texels of `triangle` of `mesh` for light slot `light`: colored
/// texels when the bake is colored, white ones otherwise.
fn bounce(baked: &BakedScene, mesh: &BakeMesh, triangle: usize, light: u32) -> Vec<Vec3> {
    let words = baked.meshes.iter().find(|m| m.id == mesh.id).unwrap();
    let corners = &mesh.indices[triangle * 3..][..3];
    let vertices = [0, 1, 2].map(|k| mesh.positions[corners[k] as usize]);
    let normals = [0, 1, 2].map(|k| mesh.normals[corners[k] as usize]);
    let uvs: [Vec2; 3] = [0, 1, 2].map(|k| mesh.uv1[corners[k] as usize]);
    let view = TriangleView::load(&words.triangles, triangle as u32, baked.settings.bounce);
    let entry = (0..view.light_count)
        .map(|k| view.light(k))
        .find(|e| e.light_index == light)
        .expect("the light reaches the triangle");
    let face = (vertices[1] - vertices[0])
        .cross(vertices[2] - vertices[0])
        .normalize();
    let texels = triangle_texels(vertices, normals, uvs, face, words.resolution);
    assert!(!texels.is_empty());
    if entry.bounce_offset == 0 {
        return vec![Vec3::ZERO; texels.len()];
    }
    texels
        .iter()
        .map(|t| {
            let stored = if baked.settings.colored_bounce {
                view.bounce_texel_rgb(&entry, t.x, t.y, 8)
            } else {
                Vec3::splat(view.bounce_texel(&entry, t.x, t.y, 8))
            };
            stored * stored
        })
        .collect()
}

fn mean(values: &[Vec3]) -> Vec3 {
    values.iter().sum::<Vec3>() / values.len() as f32
}

#[test]
fn a_sunlit_floor_bounces_onto_a_wall_by_its_view_factor() {
    let input = sun_input(Vec3::ONE);
    let baked = bake(&input, &options(false), &BakeProgress::new()).unwrap();
    assert!(baked.settings.bounce && !baked.settings.colored_bounce);
    assert_eq!(
        baked.lights[0].bounce_compression,
        Some(BounceCompression::Bits8)
    );
    let wall = &input.meshes[1];
    let values: Vec<Vec3> = (0..2).flat_map(|t| bounce(&baked, wall, t, 0)).collect();
    // From a point h up a wall, a floor reaching L = 10 m out fills a view factor of
    // ½ (1 − h / √(h² + L²)) ≈ 0.48 at h = 0.5; the floor's finite width takes a little more.
    let average = mean(&values).x;
    assert!((0.40..0.50).contains(&average), "{average}");
    assert!(
        values.iter().all(|v| v.x > 0.3),
        "every texel sees the floor"
    );
    // The floor is lit straight on and sees only the thin wall: almost no bounce.
    let ground = mean(&bounce(&baked, &input.meshes[0], 0, 0)).x;
    assert!(ground < 0.01, "{ground}");
}

#[test]
fn colored_sun_bounce_takes_the_floor_color() {
    let input = sun_input(RED);
    let baked = bake(&input, &options(true), &BakeProgress::new()).unwrap();
    assert!(baked.settings.colored_bounce);
    let wall = &input.meshes[1];
    let colored = mean(
        &(0..2)
            .flat_map(|t| bounce(&baked, wall, t, 0))
            .collect::<Vec<_>>(),
    );
    let white = bake(&sun_input(Vec3::ONE), &options(false), &BakeProgress::new()).unwrap();
    let white = mean(
        &(0..2)
            .flat_map(|t| bounce(&white, wall, t, 0))
            .collect::<Vec<_>>(),
    );
    let expected = RED * white.x;
    assert!(
        colored.abs_diff_eq(expected, 0.01),
        "{colored} vs {expected}"
    );
}

#[test]
fn photons_carry_the_color_of_what_they_hit() {
    let mut light = point_light(10, Vec3::new(0.0, 1.5, 0.0), 5.0);
    light.illumination = Illumination::SingleBounce;
    light.bounce.samples = 12;
    let mut ground = floor(1, Vec3::ZERO, 3.0);
    ground.albedo = RED;
    let input = BakeInput {
        meshes: vec![
            ground,
            cuboid(2, Vec3::new(1.0, 0.0, -0.5), Vec3::new(1.5, 1.0, 0.5)),
        ],
        lights: vec![light],
    };
    let baked = bake(&input, &options(true), &BakeProgress::new()).unwrap();
    // The box's far side sees only the red floor.
    let values: Vec<Vec3> = (0..2)
        .flat_map(|t| bounce(&baked, &input.meshes[1], t, 0))
        .collect();
    let lit: Vec<&Vec3> = values.iter().filter(|v| v.x > 0.0).collect();
    assert!(lit.len() * 2 > values.len());
    for v in lit {
        // 8 bits per channel: blue stays 0, green follows red at an eighth, within a step.
        assert_eq!(v.z, 0.0, "{v}");
        assert!((v.y - v.x / 8.0).abs() < 0.01, "{v}");
    }
}

#[test]
fn white_bounce_ignores_colors() {
    let red = encode(&bake(&sun_input(RED), &options(false), &BakeProgress::new()).unwrap());
    let white =
        encode(&bake(&sun_input(Vec3::ONE), &options(false), &BakeProgress::new()).unwrap());
    assert_eq!(red, white);
}

#[test]
fn colored_bakes_are_byte_identical_on_any_thread_count() {
    let input = sun_input(RED);
    let run = |threads| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| encode(&bake(&input, &options(true), &BakeProgress::new()).unwrap()))
    };
    let single = run(1);
    assert_eq!(single, run(3));
    assert_eq!(single, run(8));
}
