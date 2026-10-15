//! Ray queries: two-sided hits, inclusive maximum distance, alpha-tested transparency.

use bevy::math::{Vec2, Vec3, Vec3A};

use super::floor;
use crate::bake::input::{AlphaTexture, BakeMesh};
use crate::bake::occluder::{Occluders, RayCaster, intersect};

fn quad() -> [Vec3A; 3] {
    [Vec3A::ZERO, Vec3A::X, Vec3A::Z]
}

#[test]
fn hits_are_two_sided_and_include_the_maximum() {
    let origin = Vec3A::new(0.25, 1.0, 0.25);
    let (t, w) = intersect(&quad(), origin, Vec3A::NEG_Y, 10.0).unwrap();
    assert_eq!(t, 1.0);
    assert!((w.x + w.y + w.z - 1.0).abs() < 1e-6);
    let below = Vec3A::new(0.25, -1.0, 0.25);
    assert!(
        intersect(&quad(), below, Vec3A::Y, 10.0).is_some(),
        "back face"
    );
    assert!(
        intersect(&quad(), origin, Vec3A::NEG_Y, 1.0).is_some(),
        "t == max"
    );
    assert!(intersect(&quad(), origin, Vec3A::NEG_Y, 0.999).is_none());
    assert!(
        intersect(&quad(), origin, Vec3A::Y, 10.0).is_none(),
        "behind the ray"
    );
    assert!(
        intersect(&quad(), origin, Vec3A::X, 10.0).is_none(),
        "parallel"
    );
}

#[test]
fn bvh_queries_match_the_geometry() {
    let plane = floor(1, Vec3::ZERO, 1.0);
    let meshes = [&plane];
    let occluders = Occluders::build(&meshes);
    let origin = Vec3::new(0.3, 2.0, -0.4);
    assert!(occluders.occluded(origin, Vec3::NEG_Y, 2.0, false));
    assert!(!occluders.occluded(origin, Vec3::NEG_Y, 1.99, false));
    assert!(!occluders.occluded(origin, Vec3::X, 100.0, false));
    assert!(occluders.occluded(Vec3::new(0.3, -1.0, 0.0), Vec3::Y, 5.0, false));
    let hit = occluders
        .closest_hit(origin, Vec3::NEG_Y, 10.0, false)
        .unwrap();
    assert!((hit.distance - 2.0).abs() < 1e-6);
    assert_eq!(occluders.surface_normal(&hit), Vec3::Y);
    assert!(
        occluders
            .closest_hit(origin, Vec3::Y, 10.0, false)
            .is_none()
    );
    // Two stacked planes: the closest hit is the upper one.
    let upper = floor(2, Vec3::Y, 1.0);
    let meshes = [&plane, &upper];
    let occluders = Occluders::build(&meshes);
    let hit = occluders
        .closest_hit(origin, Vec3::NEG_Y, 10.0, false)
        .unwrap();
    assert!((hit.distance - 1.0).abs() < 1e-6);
    assert_eq!(occluders.triangles[hit.occluder as usize].mesh, 1);
}

/// A plane whose texture is transparent on its left half (u < 0.5).
fn half_transparent() -> BakeMesh {
    let mut plane = floor(1, Vec3::ZERO, 1.0);
    plane.uv0 = Some(plane.uv1.clone());
    plane.alpha = Some(AlphaTexture {
        width: 2,
        height: 1,
        rgba8: vec![255, 255, 255, 0, 255, 255, 255, 255],
        cutoff: AlphaTexture::DEFAULT_CUTOFF,
    });
    plane
}

#[test]
fn alpha_tested_rays_pass_transparent_texels() {
    let plane = half_transparent();
    plane.validate().unwrap();
    let meshes = [&plane];
    let occluders = Occluders::build(&meshes);
    let left = Vec3::new(-0.5, 1.0, 0.0);
    let right = Vec3::new(0.5, 1.0, 0.0);
    assert!(!occluders.occluded(left, Vec3::NEG_Y, 2.0, true));
    assert!(occluders.occluded(right, Vec3::NEG_Y, 2.0, true));
    assert!(
        occluders.occluded(left, Vec3::NEG_Y, 2.0, false),
        "opaque without the test"
    );
    assert!(
        occluders
            .closest_hit(left, Vec3::NEG_Y, 2.0, true)
            .is_none()
    );
    let texture = plane.alpha.as_ref().unwrap();
    assert!(!texture.is_opaque(Vec2::new(0.25, 0.5)));
    assert!(texture.is_opaque(Vec2::new(1.75, -3.5)), "repeats");
}
