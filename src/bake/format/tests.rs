use super::*;
use crate::light::{AreaLightShape, LightType};

fn sun(id: u64) -> BakedLight {
    BakedLight {
        id: BakeId(id),
        origin: Vec3::ZERO,
        radius: 10.0,
        shape: LightShape::Sun(SunDisk {
            rotation: Quat::from_rotation_x(-1.0),
            angular_diameter: 0.53,
        }),
        bounce_compression: None,
        shadow_bits_per_texel: 4,
    }
}

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
                shape: LightShape::Area(AreaLightShape {
                    kind: LightType::Rectangle,
                    rotation: Quat::from_rotation_x(-0.5),
                    size: Vec2::new(1.6, 0.4),
                }),
                bounce_compression: Some(BounceCompression::Bits5),
                shadow_bits_per_texel: 4,
            },
            BakedLight {
                id: BakeId(7),
                origin: Vec3::ZERO,
                radius: 0.5,
                shape: LightShape::Point,
                bounce_compression: None,
                shadow_bits_per_texel: 1,
            },
            BakedLight {
                id: BakeId(9),
                origin: Vec3::Y,
                radius: 3.0,
                shape: LightShape::Area(AreaLightShape {
                    kind: LightType::Tube,
                    rotation: Quat::IDENTITY,
                    size: Vec2::new(2.0, 0.0),
                }),
                bounce_compression: None,
                shadow_bits_per_texel: 2,
            },
            sun(3),
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
        sun_maps: vec![SunMap {
            rows: [Vec4::X, Vec4::new(0.0, 0.5, 0.0, 0.25), Vec4::splat(-0.125)],
            resolution: 3,
            texels: vec![0xFFFF_0000, 1, 2, 3, 0x8000],
        }],
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
    assert_eq!((scene.local_light_count(), scene.sun_count()), (3, 1));
}

#[test]
fn suns_come_last_with_one_sun_map_each_or_none() {
    let mut early_sun = sample();
    early_sun.lights.insert(0, sun(4));
    assert_eq!(
        decode_payload(&encode_payload(&early_sun)),
        Err(FormatError::Invalid("light order"))
    );
    let mut two_suns = sample();
    two_suns.lights.push(sun(5));
    assert_eq!(
        decode_payload(&encode_payload(&two_suns)),
        Err(FormatError::Invalid("sun map count"))
    );
    two_suns.sun_maps.clear();
    assert!(
        decode_payload(&encode_payload(&two_suns)).is_ok(),
        "no maps"
    );
    let mut short = sample();
    short.sun_maps[0].texels.pop();
    assert_eq!(
        decode_payload(&encode_payload(&short)),
        Err(FormatError::Invalid("sun map"))
    );
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
    bytes[8..12].copy_from_slice(&1u32.to_le_bytes());
    assert!(
        matches!(
            decode(&bytes),
            Err(FormatError::UnsupportedVersion { found: 1, .. })
        ),
        "version 1 bakes have no light shapes and need a re-bake"
    );
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
    // The first light's shape follows 20 bytes of settings, the light count and 28 bytes of id,
    // origin, radius and bounce precision.
    let shape = 20 + 8 + 28;
    let mut payload = encode_payload(&sample());
    payload[shape] = 7;
    assert_eq!(
        decode_payload(&payload),
        Err(FormatError::Invalid("light shape"))
    );
    for bits in [0, 3, 8] {
        let mut payload = encode_payload(&sample());
        payload[shape + 28] = bits;
        assert_eq!(
            decode_payload(&payload),
            Err(FormatError::Invalid("shadow bits per texel"))
        );
    }
    let mut payload = encode_payload(&sample());
    payload.push(0);
    assert_eq!(
        decode_payload(&payload),
        Err(FormatError::Invalid("trailing payload"))
    );
}
