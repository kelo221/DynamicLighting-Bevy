//! The `dynamic_triangles` layout round-trips through the shader-side lookup.

use bevy::math::Vec3;

use super::{cuboid, floor, point_light};
use crate::bake::bits::{BitGrid, RegionBits, TexelRect};
use crate::bake::input::{BakeInput, BakeOptions};
use crate::bake::mesh_builder::{PreparedMesh, crop_rect};
use crate::bake::mirror::TriangleView;
use crate::bake::progress::BakeProgress;
use crate::bake::shadows::crop_shadow_bits;
use crate::bake::triangle_words::{TriangleLight, TriangleRecord, serialize_triangles};
use crate::bake::{bake, decode, encode};

#[test]
fn every_padded_texel_reads_back_through_the_mirror() {
    // A map region with a pseudo-random bit pattern, cropped for triangles at the map edges
    // (x = 0, y = 0) and in the middle.
    let size = 24;
    let rect = TexelRect {
        min_x: 0,
        min_y: 0,
        max_x: size - 1,
        max_y: size - 1,
    };
    let mut grid = BitGrid::new(size, size);
    for i in 0..size * size {
        if crate::effects::splitmix64(u64::from(i)).is_multiple_of(3) {
            grid.set(i % size, i / size, true);
        }
    }
    let region = RegionBits { rect, grid };
    let crops = [
        TexelRect {
            min_x: 0,
            min_y: 0,
            max_x: 4,
            max_y: 2,
        },
        TexelRect {
            min_x: 7,
            min_y: 3,
            max_x: 15,
            max_y: 20,
        },
        TexelRect {
            min_x: 20,
            min_y: 0,
            max_x: 23,
            max_y: 23,
        },
    ];
    let records: Vec<TriangleRecord> = crops
        .iter()
        .enumerate()
        .map(|(i, &crop)| TriangleRecord {
            crop,
            lights: vec![TriangleLight {
                light: i as u32 * 3,
                shadow: Some(crop_shadow_bits(&region, crop).into_words()),
                bounce: None,
            }],
        })
        .collect();
    for bounce_layout in [false, true] {
        let words = serialize_triangles(&records, bounce_layout);
        for (triangle, crop) in crops.iter().enumerate() {
            let expected = crop_shadow_bits(&region, *crop);
            let view = TriangleView::load(&words, triangle as u32, bounce_layout);
            let entry = view.light(0);
            assert_eq!(entry.light_index, triangle as u32 * 3);
            for y in 0..expected.height() {
                for x in 0..expected.width() {
                    // Shadow map coordinates of padded texel (x, y), possibly "negative".
                    let map_x = (crop.min_x + x).wrapping_sub(2);
                    let map_y = (crop.min_y + y).wrapping_sub(2);
                    assert_eq!(
                        view.shadow_bit(&entry, map_x, map_y),
                        Some(expected.get(x, y)),
                        "triangle {triangle} texel ({x}, {y})"
                    );
                }
            }
            // Interior texels are the map's own bits.
            let (cx, cy) = (crop.min_x + 1, crop.min_y + 1);
            let own = region.get(i64::from(cx), i64::from(cy));
            assert_eq!(view.shadow_bit(&entry, cx, cy), Some(own));
        }
    }
}

#[test]
fn baked_words_are_well_formed() {
    let input = BakeInput {
        meshes: vec![
            floor(1, Vec3::ZERO, 2.0),
            cuboid(2, Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5)),
        ],
        lights: vec![
            point_light(1, Vec3::new(1.0, 2.0, 0.0), 4.0),
            point_light(2, Vec3::new(-1.0, 0.5, 1.0), 2.0),
        ],
    };
    let options = BakeOptions {
        pixel_density: 10.0,
        ..Default::default()
    };
    let baked = bake(&input, &options, &BakeProgress::new()).unwrap();
    assert_eq!(
        decode(&encode(&baked)),
        Ok(baked.clone()),
        "file round trip"
    );

    for (mesh, source) in baked.meshes.iter().zip(&input.meshes) {
        let prepared = PreparedMesh::new(source, &options);
        assert_eq!(mesh.resolution, prepared.resolution);
        let words = &mesh.triangles;
        let triangles = prepared.triangle_count() as u32;
        assert_eq!(mesh.triangle_count(), triangles);
        let mut next_data = None;
        for triangle in 0..triangles {
            let view = TriangleView::load(words, triangle, baked.settings.bounce);
            let crop = crop_rect(prepared.uvs(triangle as usize), mesh.resolution);
            assert_eq!(view.bounds[0], crop.min_x.wrapping_sub(2));
            assert_eq!(view.bounds[1], crop.min_y.wrapping_sub(2));
            assert_eq!(view.bounds[2], crop.width() + 4);
            let shadow_words = (crop.width() + 4) * (crop.height() + 4);
            for k in 0..view.light_count {
                let entry = view.light(k);
                assert!((entry.light_index as usize) < baked.lights.len());
                if k > 0 {
                    assert!(view.light(k - 1).light_index < entry.light_index);
                }
                assert_eq!(entry.bounce_offset, 0);
                // Payloads follow each other without gaps, in triangle and light order.
                let start = entry.shadow_offset;
                assert_eq!(next_data.unwrap_or(start), start);
                next_data = Some(start + shadow_words.div_ceil(32));
            }
        }
        assert_eq!(
            next_data.map_or(words.len(), |end| end as usize),
            words.len()
        );
    }
    // Each triangle's light list is ascending, and the box's bottom face (facing down, away
    // from both lights above it) has none.
    let cube = &baked.meshes[1];
    for triangle in 6..8 {
        assert_eq!(
            TriangleView::load(&cube.triangles, triangle, false).light_count,
            0
        );
    }
}
