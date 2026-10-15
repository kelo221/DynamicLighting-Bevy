use super::*;

fn sample() -> BakedScene {
    BakedScene {
        settings: BakedSettings {
            bounce: true,
            dynamic_geometry: DynamicGeometryMode::DistanceCubes,
            pixel_density: 64.0,
            max_lightmap_size: 1024,
            distance_cube_resolution: 64,
        },
        lights: vec![
            BakedLight {
                id: BakeId(42),
                origin: Vec3::new(1.0, -2.5, 3.25),
                radius: 6.0,
                bounce_compression: Some(BounceCompression::Bits5),
            },
            BakedLight {
                id: BakeId(7),
                origin: Vec3::ZERO,
                radius: 0.5,
                bounce_compression: None,
            },
        ],
        light_bvh: vec![LightBvhNode {
            aabb_min: [-1.0, -2.0, -3.0],
            left_first: 0,
            aabb_max: [4.0, 5.0, 6.0],
            count: 2,
        }],
        meshes: vec![
            BakedMesh {
                id: BakeId(1),
                content_hash: 0xABCD,
                resolution: 128,
                triangles: vec![4, 0, 0, 5, 0],
            },
            BakedMesh {
                id: BakeId(2),
                content_hash: 3,
                resolution: 2,
                triangles: vec![],
            },
        ],
        distance_cubes: vec![0x3C00_3C00, 7],
        stats: BakeStats {
            meshes: 2,
            triangles: 1,
            texels: 10,
            rays: 20,
            lights_removed: 1,
            bounce_rays: 0,
            triangle_words: 5,
        },
    }
}

#[test]
fn scenes_round_trip() {
    let scene = sample();
    let bytes = encode(&scene);
    assert_eq!(&bytes[..8], b"DYNLIGHT");
    assert_eq!(decode(&bytes), Ok(scene.clone()));
    assert_eq!(encode(&scene), bytes, "encoding is deterministic");
    let empty = BakedScene::default();
    assert_eq!(decode(&encode(&empty)), Ok(empty));
}

#[test]
fn unknown_versions_are_rejected_clearly() {
    let mut bytes = encode(&sample());
    bytes[8..12].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
    let error = decode(&bytes).unwrap_err();
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
fn corruption_is_detected() {
    let bytes = encode(&sample());
    assert_eq!(
        decode(b"PNG\0\0\0\0\0garbage-garbage-garbage"),
        Err(FormatError::NotDynlight)
    );
    assert_eq!(decode(&bytes[..10]), Err(FormatError::Truncated));
    assert!(decode(&bytes[..bytes.len() - 3]).is_err());

    let mut wrong_checksum = bytes.clone();
    wrong_checksum[20] ^= 1;
    assert_eq!(decode(&wrong_checksum), Err(FormatError::ChecksumMismatch));

    let mut wrong_length = bytes.clone();
    wrong_length[12] ^= 1;
    assert!(decode(&wrong_length).is_err());

    let mut flipped = bytes;
    let last = flipped.len() - 12;
    flipped[last] ^= 0x55;
    assert!(decode(&flipped).is_err());
}

#[test]
fn invalid_payload_values_are_rejected() {
    let mut scene = sample();
    scene.settings.max_lightmap_size = 1;
    let mut payload = encode_payload(&scene);
    payload[4] = 9; // dynamic geometry mode
    assert_eq!(
        decode_payload(&payload),
        Err(FormatError::Invalid("dynamic geometry mode"))
    );
    let mut payload = encode_payload(&sample());
    payload.push(0);
    assert_eq!(
        decode_payload(&payload),
        Err(FormatError::Invalid("trailing payload"))
    );
}
