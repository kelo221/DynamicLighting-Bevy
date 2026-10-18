//! Bake tests on synthetic scenes: analytic shadow footprints, determinism, and the word layout
//! read back through the shader mirror.

mod bounce;
mod determinism;
mod footprint;
mod layout;
mod occluder;
mod sky;
mod sun;

use bevy::math::{Quat, Vec2, Vec3};

use crate::light::{AreaLightShape, Illumination, LightShape, LightType, SunDisk, Transparency};
use crate::shared::{BakeId, mesh_content_hash};

use super::input::{BakeBounce, BakeLight, BakeMesh};

/// A horizontal square of side `2 · half` centered at `center`, facing +Y, UV1 spanning it.
pub fn floor(id: u64, center: Vec3, half: f32) -> BakeMesh {
    let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    let positions: Vec<Vec3> = corners
        .iter()
        .map(|&(x, z)| center + Vec3::new(x * half, 0.0, z * half))
        .collect();
    let uv1: Vec<Vec2> = corners
        .iter()
        .map(|&(x, z)| Vec2::new((x + 1.0) * 0.5, (z + 1.0) * 0.5))
        .collect();
    mesh(id, positions, vec![Vec3::Y; 4], vec![0, 2, 1, 0, 3, 2], uv1)
}

/// An axis-aligned box with outward faces; each face gets its own cell of a 3×2 UV1 atlas.
pub fn cuboid(id: u64, min: Vec3, max: Vec3) -> BakeMesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let mut uv1 = Vec::new();
    let faces = [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z];
    for (face, normal) in faces.into_iter().enumerate() {
        // Two tangents so that (u, v, normal) is right-handed: corners wind counter-clockwise
        // seen from outside.
        let u = if normal.y.abs() > 0.5 {
            Vec3::X
        } else {
            Vec3::Y
        };
        let v = normal.cross(u);
        let base = positions.len() as u32;
        let cell = Vec2::new((face % 3) as f32, (face / 3) as f32);
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let local = normal + u * a + v * b;
            let point = (min + max) * 0.5 + local * (max - min) * 0.5;
            positions.push(point);
            normals.push(normal);
            let inner = Vec2::new(a + 1.0, b + 1.0) * 0.5 * 0.8 + 0.1;
            uv1.push((cell + inner) / Vec2::new(3.0, 2.0));
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh(id, positions, normals, indices, uv1)
}

fn mesh(
    id: u64,
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    indices: Vec<u32>,
    uv1: Vec<Vec2>,
) -> BakeMesh {
    let p: Vec<[f32; 3]> = positions.iter().map(|p| p.to_array()).collect();
    let uv: Vec<[f32; 2]> = uv1.iter().map(|u| u.to_array()).collect();
    BakeMesh {
        id: BakeId(id),
        content_hash: mesh_content_hash(&p, &indices, &uv),
        positions,
        normals,
        indices,
        uv1,
        uv0: None,
        alpha: None,
    }
}

pub fn point_light(id: u64, position: Vec3, radius: f32) -> BakeLight {
    BakeLight {
        id: BakeId(id),
        position,
        radius,
        illumination: Illumination::Direct,
        transparency: Transparency::Disabled,
        bounce: BakeBounce::default(),
        shape: LightShape::Point,
    }
}

/// A rectangle (`size.y > 0`) or tube light turned by `rotation`.
pub fn area_light(id: u64, position: Vec3, radius: f32, rotation: Quat, size: Vec2) -> BakeLight {
    let kind = if size.y > 0.0 {
        LightType::Rectangle
    } else {
        LightType::Tube
    };
    BakeLight {
        shape: LightShape::Area(AreaLightShape {
            kind,
            rotation,
            size,
        }),
        ..point_light(id, position, radius)
    }
}

/// A sun `degrees` wide in direction `toward` (from the scene), far away from the origin to
/// show that its position doesn't count.
pub fn sun_light(id: u64, toward: Vec3, degrees: f32) -> BakeLight {
    BakeLight {
        shape: LightShape::Sun(SunDisk {
            rotation: Quat::from_rotation_arc(Vec3::Z, toward.normalize()),
            angular_diameter: degrees,
        }),
        ..point_light(id, Vec3::new(500.0, -300.0, 70.0), 0.5)
    }
}

/// A baked sky. Its position and radius don't count either.
pub fn sky_light(id: u64) -> BakeLight {
    BakeLight {
        shape: LightShape::Sky,
        ..point_light(id, Vec3::new(-40.0, 900.0, 3.0), 0.5)
    }
}
