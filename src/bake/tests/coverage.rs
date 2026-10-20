//! Which texels get traced: thin strips that hold no texel center (a thin pipe's UV islands) are
//! still lit, and a floor that runs through a box (a pipe through a beam) doesn't bake black at
//! the box's edge, where its texel centers lie inside the box.

use bevy::math::{Vec2, Vec3};

use super::{cuboid, floor, mesh, point_light};
use crate::bake::geometry::lightmap_size;
use crate::bake::input::{BakeInput, BakeOptions};
use crate::bake::mirror::TriangleView;
use crate::bake::progress::BakeProgress;
use crate::bake::{BakedScene, bake};
use crate::shared::{BakeId, DynamicGeometryMode};

const HALF: f32 = 1.0;
const BOX: f32 = 0.25;

fn bake_floor_through_box(box_bottom: f32) -> BakedScene {
    let input = BakeInput {
        meshes: vec![
            floor(1, Vec3::ZERO, HALF),
            cuboid(2, Vec3::new(-BOX, box_bottom, -BOX), Vec3::splat(BOX)),
        ],
        // Far off to +X, so the floor next to the box's +X side is lit.
        lights: vec![point_light(10, Vec3::new(3.0, 1.5, 0.0), 8.0)],
    };
    let options = BakeOptions {
        pixel_density: 16.0,
        dynamic_geometry: DynamicGeometryMode::LightingOnly,
        ..Default::default()
    };
    bake(&input, &options, &BakeProgress::new()).unwrap()
}

/// Shadow bits of the floor texels at the box's +X side whose centers lie within 1.5 texels
/// inside the box: (lit, dark).
fn edge_texels(baked: &BakedScene) -> (u32, u32) {
    let mesh = baked.meshes.iter().find(|m| m.id == BakeId(1)).unwrap();
    let size = mesh.resolution;
    let texel = 2.0 * HALF / size as f32;
    let (mut lit, mut dark) = (0, 0);
    for triangle in 0..2 {
        let view = TriangleView::load(&mesh.triangles, triangle, false);
        let entry = view.light(0);
        for y in 0..size {
            for x in 0..size {
                // `floor` maps UV1 (0, 0) to (-HALF, -HALF) and UV1 v to +Z.
                let uv = (Vec2::new(x as f32, y as f32) + 0.5) / size as f32;
                let at = (uv * 2.0 - 1.0) * HALF;
                let inside = at.x < BOX && at.x > BOX - 1.5 * texel && at.y.abs() < BOX * 0.6;
                match view.shadow_bit(&entry, x, y) {
                    Some(true) if inside => lit += 1,
                    Some(false) if inside => dark += 1,
                    _ => {}
                }
            }
        }
    }
    (lit, dark)
}

#[test]
fn texels_inside_a_box_take_their_visible_neighbours_light() {
    let (lit, dark) = edge_texels(&bake_floor_through_box(-BOX));
    assert!(lit > 0 && dark == 0, "{lit} lit, {dark} dark");
}

#[test]
fn texels_under_a_box_standing_on_the_floor_do_too() {
    let (lit, dark) = edge_texels(&bake_floor_through_box(0.0));
    assert!(lit > 0 && dark == 0, "{lit} lit, {dark} dark");
}

/// Six floor strips 0.1 m wide whose UV islands are 0.4 texels wide and 3 texels apart, so none
/// holds a texel center.
#[test]
fn strips_narrower_than_a_texel_are_lit() {
    const STRIPS: usize = 6;
    let density = 32.0;
    let size = lightmap_size(STRIPS as f32 * 0.1, density, 2048);
    let (mut positions, mut uv1, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    for k in 0..STRIPS {
        let x = k as f32 * 0.1 - 0.3;
        let u = (3.0 * k as f32 + 0.55) / size as f32;
        let du = 0.4 / size as f32;
        let base = positions.len() as u32;
        for (a, b) in [(0.0, -0.5), (0.1, -0.5), (0.1, 0.5), (0.0, 0.5)] {
            positions.push(Vec3::new(x + a, 0.0, b));
            uv1.push(Vec2::new(u + a * 10.0 * du, 0.5 + b * 0.9));
        }
        indices.extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    let normals = vec![Vec3::Y; positions.len()];
    let input = BakeInput {
        meshes: vec![mesh(1, positions, normals, indices, uv1.clone())],
        lights: vec![point_light(10, Vec3::new(0.2, 1.5, 0.1), 8.0)],
    };
    let options = BakeOptions {
        pixel_density: density,
        dynamic_geometry: DynamicGeometryMode::LightingOnly,
        ..Default::default()
    };
    let baked = bake(&input, &options, &BakeProgress::new()).unwrap();
    let strips = &baked.meshes[0];
    assert_eq!(strips.resolution, size);
    for triangle in 0..2 * STRIPS as u32 {
        let view = TriangleView::load(&strips.triangles, triangle, false);
        assert_eq!(view.light_count, 1, "triangle {triangle} kept its light");
        let corners = &input.meshes[0].indices[triangle as usize * 3..][..3];
        let centroid = corners.iter().map(|&c| uv1[c as usize]).sum::<Vec2>() / 3.0;
        let texel = (centroid * size as f32).floor();
        let lit = view.shadow_bit(&view.light(0), texel.x as u32, texel.y as u32);
        assert_eq!(lit, Some(true), "triangle {triangle} at texel {texel}");
    }
}

/// One mesh whose UV1 islands overlap, like a cylinder's caps over its side: a lit floor and a
/// second quad with the same UVs shut inside a box below it. The quad's texels are buried; the
/// floor's, at the same texels, must still be traced.
#[test]
fn a_buried_island_leaves_an_overlapping_one_lit() {
    let lit_floor = floor(1, Vec3::ZERO, 0.5);
    let hidden = floor(1, Vec3::Y * -0.75, 0.25);
    let offset = lit_floor.positions.len() as u32;
    let mut both = lit_floor.clone();
    both.positions.extend(&hidden.positions);
    both.normals.extend(&hidden.normals);
    both.uv1.extend(&hidden.uv1);
    both.indices
        .extend(hidden.indices.iter().map(|i| i + offset));
    let input = BakeInput {
        meshes: vec![
            mesh(1, both.positions, both.normals, both.indices, both.uv1),
            cuboid(2, Vec3::new(-0.4, -1.0, -0.4), Vec3::new(0.4, -0.5, 0.4)),
        ],
        lights: vec![point_light(10, Vec3::new(0.1, 1.5, 0.2), 8.0)],
    };
    let options = BakeOptions {
        pixel_density: 16.0,
        dynamic_geometry: DynamicGeometryMode::LightingOnly,
        ..Default::default()
    };
    let baked = bake(&input, &options, &BakeProgress::new()).unwrap();
    let floor_mesh = baked.meshes.iter().find(|m| m.id == BakeId(1)).unwrap();
    let (mut lit, mut dark) = (0, 0);
    for triangle in 0..2 {
        let view = TriangleView::load(&floor_mesh.triangles, triangle, false);
        let entry = view.light(0);
        for y in 0..floor_mesh.resolution {
            for x in 0..floor_mesh.resolution {
                match view.shadow_bit(&entry, x, y) {
                    Some(true) => lit += 1,
                    Some(false) => dark += 1,
                    None => {}
                }
            }
        }
    }
    assert!(lit > 0 && dark == 0, "{lit} lit, {dark} dark");
}
