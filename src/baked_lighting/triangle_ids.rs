//! Triangle IDs as a vertex attribute, for GPUs without `@builtin(primitive_index)`.
//!
//! Baked data is keyed on each triangle's position in the mesh's draw order. Without primitive
//! indices, baked meshes are de-indexed and every corner carries the index of its triangle;
//! `DynLitMaterial` then draws them with its own vertex shader that passes the ID through.

use bevy::mesh::{MeshVertexAttribute, PrimitiveTopology, VertexFormat};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;

use super::features::primitive_index_missing;
use crate::shared::{DynamicLightingSettings, ResolvedBakeId, TriangleIdSource};

/// The index of the triangle a corner belongs to, in the original draw order.
pub const ATTRIBUTE_TRIANGLE_ID: MeshVertexAttribute = MeshVertexAttribute::new(
    "DynlightTriangleId",
    0x7c3e_91a5_d24b_6f08,
    VertexFormat::Uint32,
);

/// Shader location of [`ATTRIBUTE_TRIANGLE_ID`] in `DynLitMaterial`'s vertex shader, and of the
/// triangle ID it passes to the fragment shader. Bevy's mesh vertex layout uses 0 to 7.
pub const TRIANGLE_ID_LOCATION: u32 = 8;

/// Whether `mesh` carries [`ATTRIBUTE_TRIANGLE_ID`].
pub fn has_triangle_ids(mesh: &Mesh) -> bool {
    matches!(
        mesh.try_attribute_option(ATTRIBUTE_TRIANGLE_ID),
        Ok(Some(_))
    )
}

/// Whether [`add_triangle_ids`] can work on `mesh`: a triangle list whose vertex data is still
/// in the main world.
pub fn accepts_triangle_ids(mesh: &Mesh) -> bool {
    mesh.primitive_topology() == PrimitiveTopology::TriangleList
        && mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION).is_ok()
}

/// De-indexes `mesh` and adds [`ATTRIBUTE_TRIANGLE_ID`]. The triangles, and so the mesh's
/// content hash, stay the same. Returns false, leaving the mesh as it was, when
/// [`accepts_triangle_ids`] is false.
pub fn add_triangle_ids(mesh: &mut Mesh) -> bool {
    if !accepts_triangle_ids(mesh) {
        return false;
    }
    if has_triangle_ids(mesh) {
        return true;
    }
    if mesh.try_duplicate_vertices().is_err() {
        return false;
    }
    let corners = mesh.count_vertices() as u32;
    let ids: Vec<u32> = (0..corners).map(|corner| corner / 3).collect();
    mesh.try_insert_attribute(ATTRIBUTE_TRIANGLE_ID, ids)
        .is_ok()
}

/// Gives the meshes of baked entities triangle IDs when the GPU has no primitive indices, or
/// when [`TriangleIdSource::VertexAttribute`] asks for them.
pub(super) fn attach_triangle_ids(
    settings: Res<DynamicLightingSettings>,
    receivers: Query<&Mesh3d, With<ResolvedBakeId>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut skipped: Local<HashSet<AssetId<Mesh>>>,
) {
    let wanted = match settings.triangle_ids {
        TriangleIdSource::Auto => primitive_index_missing(),
        TriangleIdSource::VertexAttribute => true,
    };
    if !wanted {
        return;
    }
    for mesh in &receivers {
        let id = mesh.id();
        let Some(current) = meshes.get(id) else {
            continue;
        };
        if skipped.contains(&id) || has_triangle_ids(current) {
            continue;
        }
        if !accepts_triangle_ids(current) {
            warn!(
                "a baked mesh isn't a triangle list or keeps no CPU copy of its vertices, so it \
                 can't get triangle IDs and is lit without baked shadows"
            );
            skipped.insert(id);
            continue;
        }
        let Some(mut mesh) = meshes.get_mut(id) else {
            continue;
        };
        if add_triangle_ids(&mut mesh) {
            debug!("{id}: {} triangles got IDs", mesh.count_vertices() / 3);
        } else {
            skipped.insert(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::{Indices, VertexAttributeValues};

    use super::*;
    use crate::shared::hash_mesh;

    fn quad() -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all())
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_POSITION,
                vec![
                    [0.0, 0.0, 0.0],
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    [1.0, 1.0, 0.0],
                ],
            )
            .with_inserted_attribute(
                Mesh::ATTRIBUTE_UV_0,
                vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            )
            .with_inserted_indices(Indices::U32(vec![0, 1, 2, 2, 1, 3]))
    }

    #[test]
    fn ids_follow_the_draw_order_and_keep_the_hash() {
        let mut mesh = quad();
        let before = hash_mesh(&mesh);
        assert!(add_triangle_ids(&mut mesh));
        assert!(mesh.indices().is_none());
        assert_eq!(mesh.count_vertices(), 6);
        let Some(VertexAttributeValues::Uint32(ids)) = mesh.attribute(ATTRIBUTE_TRIANGLE_ID) else {
            panic!("triangle IDs are missing");
        };
        assert_eq!(ids, &[0, 0, 0, 1, 1, 1]);
        assert!(before.is_some());
        assert_eq!(hash_mesh(&mesh), before);
        assert!(add_triangle_ids(&mut mesh), "a second call keeps the IDs");
        assert_eq!(mesh.count_vertices(), 6);
    }

    #[test]
    fn only_triangle_lists_get_ids() {
        let mut lines = Mesh::new(PrimitiveTopology::LineList, RenderAssetUsages::all())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 2]);
        assert!(!add_triangle_ids(&mut lines));
        assert!(!has_triangle_ids(&lines));
    }
}
