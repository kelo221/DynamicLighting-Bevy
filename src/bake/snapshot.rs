//! Snapshots the ECS into plain [`BakeInput`]: world-space static meshes and baked lights.
//!
//! Meshes are entities with [`BakeStatic`], [`Mesh3d`] and [`GlobalTransform`]; lights are
//! every [`DynamicLight`] that isn't `realtime`, area lights as one light per shadow point.
//! Entities without a [`BakeId`] get one derived from their `Name` path (root first) plus, for
//! meshes, the mesh asset path.

use std::fmt;

use bevy::ecs::system::SystemParam;
use bevy::mesh::{MeshAccessError, PrimitiveTopology, VertexAttributeValues};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;

use crate::light::{DynamicLight, shadow_point_id};
use crate::shared::{BakeId, mesh_content_hash};

use super::identity::{Candidate, NamePaths, assign_ids};
use super::input::{AlphaTexture, BakeBounce, BakeInput, BakeLight, BakeMesh, PointOf};
use super::settings::BakeStatic;

/// Base color alpha for transparent baked shadows. Put it on a [`BakeStatic`] mesh that has UV0;
/// lights with `Transparency::Enabled` then shine through texels with alpha below the cutoff.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct BakeAlphaMask(pub AlphaTexture);

/// Why the ECS couldn't be turned into bake input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// The app has no `Assets<Mesh>`, so no mesh data can be read.
    NoMeshAssets,
    /// A `Mesh3d` handle points to a mesh that isn't loaded.
    MissingMesh { entity: String },
    /// A mesh can't be baked; `reason` says why.
    MeshData { entity: String, reason: String },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMeshAssets => write!(f, "the app has no mesh assets to bake"),
            Self::MissingMesh { entity } => write!(f, "mesh of {entity} isn't loaded"),
            Self::MeshData { entity, reason } => write!(f, "can't bake {entity}: {reason}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

type StaticMesh = (
    Entity,
    &'static Mesh3d,
    &'static GlobalTransform,
    Option<&'static BakeId>,
    Option<&'static BakeAlphaMask>,
);
type BakedLightEntity = (
    Entity,
    &'static DynamicLight,
    &'static GlobalTransform,
    Option<&'static BakeId>,
);

/// The ECS data a bake reads.
#[derive(SystemParam)]
pub(super) struct SceneSnapshot<'w, 's> {
    meshes: Query<'w, 's, StaticMesh, With<BakeStatic>>,
    lights: Query<'w, 's, BakedLightEntity>,
    paths: NamePaths<'w, 's>,
    mesh_assets: Option<Res<'w, Assets<Mesh>>>,
}

impl SceneSnapshot<'_, '_> {
    pub(super) fn take(&self) -> Result<BakeInput, SnapshotError> {
        let mut meshes = Vec::new();
        let mut candidates = Vec::new();
        if !self.meshes.is_empty() {
            let assets = self
                .mesh_assets
                .as_deref()
                .ok_or(SnapshotError::NoMeshAssets)?;
            // Meshes shared by many entities are reported once.
            let mut without_uv1 = HashSet::new();
            for (entity, handle, transform, id, alpha) in &self.meshes {
                let label = || self.label(entity);
                let mesh = assets
                    .get(&handle.0)
                    .ok_or_else(|| SnapshotError::MissingMesh { entity: label() })?;
                let data = MeshData::read(mesh).map_err(|reason| SnapshotError::MeshData {
                    entity: label(),
                    reason,
                })?;
                if data.uv0_as_uv1 && without_uv1.insert(handle.0.id()) {
                    warn!(
                        "dynlight bake: the mesh of {} has no UV1; using UV0 for the shadows of \
                         every entity that uses it",
                        label()
                    );
                }
                let bake = data.to_world(transform, alpha.map(|a| a.0.clone()));
                candidates.push(Candidate::mesh(
                    &self.paths,
                    entity,
                    &handle.0,
                    id.copied(),
                    bake.content_hash,
                    transform.translation(),
                ));
                meshes.push(bake);
            }
        }
        for (mesh, id) in meshes.iter_mut().zip(assign_ids(&candidates, "mesh", true)) {
            mesh.id = id;
        }

        let mut baked = Vec::new();
        let mut candidates = Vec::new();
        for (entity, light, transform, id) in &self.lights {
            if light.realtime {
                continue;
            }
            let (_, rotation, position) = transform.to_scale_rotation_translation();
            candidates.push(Candidate::light(&self.paths, entity, id.copied(), position));
            baked.push((light, position, rotation));
        }
        // An area light is baked as one point light per shadow point.
        let mut lights = Vec::new();
        let ids = assign_ids(&candidates, "light", true);
        for ((light, position, rotation), id) in baked.into_iter().zip(ids) {
            let count = light.shadow_point_count();
            for point in light.shadow_point_layout(position, rotation) {
                lights.push(BakeLight {
                    id: shadow_point_id(id, point.index, count),
                    position: point.position,
                    radius: point.radius,
                    illumination: light.illumination,
                    transparency: light.transparency,
                    bounce: BakeBounce {
                        samples: light.bounce.samples.max(2),
                        intensity: light.bounce.intensity,
                        compression: light.bounce.compression,
                    },
                    point_of: (count > 1).then_some(PointOf {
                        light: id,
                        index: point.index,
                    }),
                });
            }
        }
        Ok(BakeInput { meshes, lights })
    }

    fn label(&self, entity: Entity) -> String {
        let path = self.paths.name_path(entity);
        if path.iter().all(String::is_empty) {
            format!("entity {entity}")
        } else {
            format!("entity {entity} ({})", path.join("/"))
        }
    }
}

/// Mesh data in local space, as read from a [`Mesh`].
pub(super) struct MeshData {
    positions: Vec<[f32; 3]>,
    normals: Option<Vec<[f32; 3]>>,
    indices: Vec<u32>,
    uv1: Vec<[f32; 2]>,
    uv0: Option<Vec<[f32; 2]>>,
    /// The mesh has no UV1, so UV0 is used for the shadow maps.
    pub(super) uv0_as_uv1: bool,
}

impl MeshData {
    pub(super) fn read(mesh: &Mesh) -> Result<Self, String> {
        if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
            return Err(format!(
                "primitive topology {:?} isn't a triangle list",
                mesh.primitive_topology()
            ));
        }
        let positions = match mesh.try_attribute(Mesh::ATTRIBUTE_POSITION) {
            Ok(values) => values
                .as_float3()
                .ok_or("positions aren't Float32x3")?
                .to_vec(),
            Err(MeshAccessError::NotFound) => return Err("the mesh has no positions".into()),
            Err(error) => return Err(access_error(error)),
        };
        let normals = mesh
            .try_attribute_option(Mesh::ATTRIBUTE_NORMAL)
            .map_err(access_error)?
            .and_then(VertexAttributeValues::as_float3)
            .map(<[_]>::to_vec);
        let indices = match mesh.try_indices_option().map_err(access_error)? {
            Some(indices) => indices.iter().map(|index| index as u32).collect(),
            None => (0..positions.len() as u32).collect(),
        };
        let uv0 = uvs(mesh, Mesh::ATTRIBUTE_UV_0)?;
        let (uv1, uv0_as_uv1) = match uvs(mesh, Mesh::ATTRIBUTE_UV_1)? {
            Some(uv1) => (uv1, false),
            None => (
                uv0.clone()
                    .ok_or("the mesh has neither UV1 nor UV0 coordinates")?,
                true,
            ),
        };
        Ok(Self {
            positions,
            normals,
            indices,
            uv1,
            uv0,
            uv0_as_uv1,
        })
    }

    /// Transforms to world space. Normals use the inverse transpose; mirroring transforms swap
    /// two corners of every triangle so face normals keep pointing out of the front face.
    /// Triangle order (the runtime's primitive index) never changes.
    pub(super) fn to_world(
        &self,
        transform: &GlobalTransform,
        alpha: Option<AlphaTexture>,
    ) -> BakeMesh {
        let affine = transform.affine();
        let linear = Mat3::from(affine.matrix3);
        let determinant = linear.determinant();
        let normal_matrix = determinant
            .is_normal()
            .then(|| linear.inverse().transpose());
        let normals = match (&self.normals, normal_matrix) {
            (Some(normals), Some(matrix)) => normals
                .iter()
                .map(|&n| (matrix * Vec3::from(n)).normalize_or_zero())
                .collect(),
            _ => Vec::new(),
        };
        let mut indices = self.indices.clone();
        if determinant < 0.0 {
            for triangle in indices.as_chunks_mut::<3>().0 {
                triangle.swap(1, 2);
            }
        }
        let to_vec2 = |uvs: &[[f32; 2]]| uvs.iter().copied().map(Vec2::from).collect::<Vec<_>>();
        let uv0 = self.uv0.as_deref().map(to_vec2);
        BakeMesh {
            id: BakeId(0),
            content_hash: mesh_content_hash(&self.positions, &self.indices, &self.uv1),
            positions: self
                .positions
                .iter()
                .map(|&p| affine.transform_point3(Vec3::from(p)))
                .collect(),
            normals,
            indices,
            uv1: to_vec2(&self.uv1),
            alpha: alpha.filter(|_| uv0.is_some()),
            uv0,
        }
    }
}

fn uvs(
    mesh: &Mesh,
    attribute: bevy::mesh::MeshVertexAttribute,
) -> Result<Option<Vec<[f32; 2]>>, String> {
    match mesh.try_attribute_option(attribute).map_err(access_error)? {
        None => Ok(None),
        Some(VertexAttributeValues::Float32x2(uvs)) => Ok(Some(uvs.clone())),
        Some(_) => Err(format!("{} isn't Float32x2", attribute.name)),
    }
}

fn access_error(error: MeshAccessError) -> String {
    match error {
        MeshAccessError::ExtractedToRenderWorld => "its vertex data only exists in the render \
            world; create the mesh with `RenderAssetUsages::MAIN_WORLD` (e.g. \
            `RenderAssetUsages::default()`) so the bake can read it"
            .into(),
        MeshAccessError::NotFound => "the mesh data is missing".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrored_transforms_keep_front_faces() {
        let data = MeshData {
            positions: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            normals: Some(vec![[0.0, 1.0, 0.0]; 3]),
            indices: vec![0, 1, 2],
            uv1: vec![[0.0, 0.0], [0.0, 1.0], [1.0, 0.0]],
            uv0: None,
            uv0_as_uv1: false,
        };
        let mirror = GlobalTransform::from(Transform::from_scale(Vec3::new(1.0, -2.0, 1.0)));
        let mesh = data.to_world(&mirror, None);
        assert_eq!(mesh.indices, [0, 2, 1]);
        assert!(mesh.normals[0].abs_diff_eq(Vec3::NEG_Y, 1e-6));
        let [a, b, c] = [0, 1, 2].map(|i| mesh.positions[mesh.indices[i] as usize]);
        let face = (b - a).cross(c - a).normalize();
        assert!(face.abs_diff_eq(Vec3::NEG_Y, 1e-6), "{face}");
        let local = data.to_world(&GlobalTransform::IDENTITY, None);
        assert_eq!(mesh.content_hash, local.content_hash, "hashes local data");
    }
}
