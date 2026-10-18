//! UV1 sets for Bevy's primitive meshes, so baked shadows and bounce get their own layout.

// Each example uses a different subset.
#![allow(dead_code)]

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;

/// A box whose faces get their own cell of a 3×2 UV1 atlas. Bevy's box UVs put every face on
/// the same square, and bounce lighting is baked into one lightmap per mesh, so faces must not
/// overlap there.
pub fn box_with_lightmap_uvs(cuboid: Cuboid) -> Mesh {
    const PADDING: f32 = 0.05;
    let mut mesh = cuboid.mesh().build();
    let Some(VertexAttributeValues::Float32x2(uv0)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0) else {
        return mesh;
    };
    // Four vertices per face.
    let uv1: Vec<[f32; 2]> = uv0
        .iter()
        .enumerate()
        .map(|(vertex, [u, v])| {
            let face = (vertex / 4) as f32;
            let cell = Vec2::new(face % 3.0, (face / 3.0).floor());
            let inner = Vec2::new(*u, *v) * (1.0 - 2.0 * PADDING) + PADDING;
            ((cell + inner) / Vec2::new(3.0, 2.0)).to_array()
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    mesh
}

/// `mesh` with its UV0 copied to UV1. Baked shadows are stored per triangle, so UV0 may overlap
/// itself unless the mesh bakes bounce lighting.
pub fn with_uv0_as_uv1(mut mesh: Mesh) -> Mesh {
    if let Some(uv0) = mesh.attribute(Mesh::ATTRIBUTE_UV_0).cloned() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv0);
    }
    mesh
}
