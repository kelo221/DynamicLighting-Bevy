//! The public bake API without an app: bake plain input, round-trip the `.dynlight` bytes and
//! read the result the way the runtime will.

use bevy::math::{Vec2, Vec3};
use dynlight::bake::mirror::{DISTANCE_CUBE_WORDS, TriangleView, distance_cube_value};
use dynlight::bake::{
    BakeInput, BakeLight, BakeMesh, BakeOptions, BakeProgress, BakeStage, FORMAT_VERSION,
    FormatError, LIGHT_BVH_NODE_SIZE, LightBvhNode, bake, decode, encode,
};
use dynlight::light::{Illumination, LightShape, Transparency};
use dynlight::shared::cube::{direction_to_texel, texel_index};
use dynlight::shared::{BakeId, DynamicGeometryMode, mesh_content_hash};

/// A 4 m floor at the origin, facing +Y.
fn floor() -> BakeMesh {
    let positions = [
        [-2.0, 0.0, -2.0],
        [2.0, 0.0, -2.0],
        [2.0, 0.0, 2.0],
        [-2.0, 0.0, 2.0],
    ];
    let uv1 = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let indices = vec![0, 2, 1, 0, 3, 2];
    BakeMesh {
        id: BakeId(7),
        content_hash: mesh_content_hash(&positions, &indices, &uv1),
        positions: positions.map(Vec3::from).to_vec(),
        normals: vec![Vec3::Y; 4],
        indices,
        uv1: uv1.map(Vec2::from).to_vec(),
        ..Default::default()
    }
}

fn light(id: u64, position: Vec3) -> BakeLight {
    BakeLight {
        id: BakeId(id),
        position,
        radius: 3.0,
        illumination: Illumination::Direct,
        transparency: Transparency::Disabled,
        bounce: Default::default(),
        shape: LightShape::Point,
    }
}

fn input() -> BakeInput {
    BakeInput {
        meshes: vec![floor()],
        lights: vec![
            light(2, Vec3::new(1.0, 1.0, 0.0)),
            light(1, Vec3::new(-1.0, 1.0, 0.0)),
        ],
    }
}

fn options() -> BakeOptions {
    BakeOptions {
        pixel_density: 4.0,
        dynamic_geometry: DynamicGeometryMode::Angular,
        ..Default::default()
    }
}

#[test]
fn bake_encode_decode_round_trip() {
    let progress = BakeProgress::new();
    let scene = bake(&input(), &options(), &progress).unwrap();
    assert_eq!(progress.stage(), BakeStage::Done);
    assert_eq!(progress.fraction(), 1.0);

    let bytes = encode(&scene);
    assert_eq!(&bytes[..8], b"DYNLIGHT");
    assert_eq!(bytes[8..12], FORMAT_VERSION.to_le_bytes());
    assert_eq!(decode(&bytes).unwrap(), scene);
    assert_eq!(
        encode(&decode(&bytes).unwrap()),
        bytes,
        "encoding is canonical"
    );

    let mut future = bytes.clone();
    future[8..12].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
    let error = decode(&future).unwrap_err();
    assert_eq!(
        error,
        FormatError::UnsupportedVersion {
            found: FORMAT_VERSION + 1,
            supported: FORMAT_VERSION
        }
    );
    assert!(error.to_string().contains("re-bake"));
}

#[test]
fn baked_scene_reads_like_the_shader() {
    let scene = bake(&input(), &options(), &BakeProgress::new()).unwrap();
    assert_eq!(std::mem::size_of::<LightBvhNode>(), LIGHT_BVH_NODE_SIZE);
    assert_eq!(scene.light_bvh.len(), 1, "two lights fit one leaf");
    assert_eq!(scene.light_bvh[0].count, 2);

    // Both lights reach both floor triangles; entries are in BVH slot order.
    let mesh = &scene.meshes[0];
    assert_eq!(mesh.resolution, 16);
    assert_eq!(mesh.triangle_count(), 2);
    for triangle in 0..2 {
        let view = TriangleView::load(&mesh.triangles, triangle, false);
        assert_eq!(view.light_count, 2);
        for k in 0..2 {
            let entry = view.light(k);
            assert_eq!(entry.light_index, k);
            assert_eq!(
                view.shadow_bit(&entry, 8, 8),
                Some(true),
                "nothing blocks the floor"
            );
        }
    }

    // Angular distance cubes: exact distances, the floor 1 m below each light.
    assert_eq!(scene.distance_cubes.len(), 2 * DISTANCE_CUBE_WORDS as usize);
    for slot in 0..2 {
        let (face, x, y) = direction_to_texel(Vec3::NEG_Y, 64);
        let below = distance_cube_value(&scene.distance_cubes, slot, texel_index(face, x, y, 64));
        assert!((below - 1.0).abs() < 0.01, "{below}");
        let (face, x, y) = direction_to_texel(Vec3::Y, 64);
        let above = distance_cube_value(&scene.distance_cubes, slot, texel_index(face, x, y, 64));
        assert_eq!(above, 3.0, "misses store the radius");
    }
}
