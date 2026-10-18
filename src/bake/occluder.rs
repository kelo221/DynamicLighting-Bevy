//! The occluder BVH over every static triangle, and the ray queries the tracer needs: an
//! any-hit shadow test and a closest-hit query, both two-sided (as Henry's bake gets from
//! Unity's `queriesHitBackfaces`), with hits counted up to and including the maximum distance.
//!
//! Transparent shadows alpha-test hits against the mesh's base color alpha during traversal.

use bevy::math::{Vec2, Vec3, Vec3A};
use obvhs::BvhBuildParams;
use obvhs::bvh2::reinsertion::ReinsertionOptimizer;
use obvhs::cwbvh::CwBvh;
use obvhs::cwbvh::bvh2_to_cwbvh::bvh2_to_cwbvh;
use obvhs::ploc::PlocBuilder;
use obvhs::ray::{Ray, RayHit};
use obvhs::triangle::Triangle;

use super::input::BakeMesh;

/// Ray queries against the static scene.
pub trait RayCaster: Sync {
    /// Whether anything blocks `origin + direction · t` for `0 ≤ t ≤ max_distance`.
    /// `direction` must be normalized.
    fn occluded(&self, origin: Vec3, direction: Vec3, max_distance: f32, alpha_test: bool) -> bool;

    /// The nearest surface along the ray within `max_distance`.
    fn closest_hit(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        alpha_test: bool,
    ) -> Option<SurfaceHit>;
}

/// A closest-hit result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceHit {
    pub distance: f32,
    /// Index into [`Occluders::triangles`].
    pub occluder: u32,
    /// Weights of the triangle's three vertices at the hit.
    pub barycentrics: Vec3,
}

/// One triangle of a static mesh, in world space.
#[derive(Clone, Copy, Debug)]
pub struct OccluderTriangle {
    pub vertices: [Vec3A; 3],
    /// Index of the mesh in the sorted bake input.
    pub mesh: u32,
    /// Triangle index within that mesh.
    pub triangle: u32,
}

/// All static triangles plus their BVH.
pub struct Occluders<'a> {
    pub triangles: Vec<OccluderTriangle>,
    meshes: &'a [&'a BakeMesh],
    bvh: Option<CwBvh>,
}

/// `obvhs::cwbvh::builder::build_cwbvh_from_tris` with `medium_build()` (no pre-splits), minus
/// its build timer: `std::time::Instant` panics on `wasm32-unknown-unknown`.
fn build_cwbvh(tris: &[Triangle]) -> CwBvh {
    let config = BvhBuildParams::medium_build();
    let mut bvh2 = PlocBuilder::with_capacity(tris.len()).build(
        config.ploc_search_distance,
        tris,
        (0..tris.len() as u32).collect(),
        config.sort_precision,
        config.search_depth_threshold,
    );
    ReinsertionOptimizer::default().run(&mut bvh2, config.reinsertion_batch_ratio, None);
    bvh2_to_cwbvh(&bvh2, config.max_prims_per_leaf.clamp(1, 3), true, false)
}

impl<'a> Occluders<'a> {
    /// Collects every non-degenerate triangle of `meshes` (sorted bake order) and builds the BVH.
    /// The build is single-threaded and depends only on the input order, so it is deterministic.
    pub fn build(meshes: &'a [&'a BakeMesh]) -> Self {
        let mut triangles = Vec::new();
        for (mesh_index, mesh) in meshes.iter().enumerate() {
            for (triangle, corners) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
                let vertices = [0, 1, 2].map(|k| Vec3A::from(mesh.positions[corners[k] as usize]));
                let normal = (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]);
                if normal.length_squared() > 0.0 && normal.is_finite() {
                    triangles.push(OccluderTriangle {
                        vertices,
                        mesh: mesh_index as u32,
                        triangle: triangle as u32,
                    });
                }
            }
        }
        let bvh = (!triangles.is_empty()).then(|| {
            let tris: Vec<Triangle> = triangles
                .iter()
                .map(|t| Triangle {
                    v0: t.vertices[0],
                    v1: t.vertices[1],
                    v2: t.vertices[2],
                })
                .collect();
            build_cwbvh(&tris)
        });
        Self {
            triangles,
            meshes,
            bvh,
        }
    }

    /// The unit normal of the hit triangle's front face (counter-clockwise winding).
    pub fn face_normal(&self, hit: &SurfaceHit) -> Vec3 {
        let [a, b, c] = self.triangles[hit.occluder as usize]
            .vertices
            .map(Vec3::from);
        (b - a).cross(c - a).normalize_or_zero()
    }

    /// The interpolated vertex normal at a hit (the face normal for meshes without normals),
    /// normalized.
    pub fn surface_normal(&self, hit: &SurfaceHit) -> Vec3 {
        let occluder = &self.triangles[hit.occluder as usize];
        let face = self.face_normal(hit);
        let mesh = self.meshes[occluder.mesh as usize];
        if mesh.normals.is_empty() {
            return face;
        }
        let corners = &mesh.indices[occluder.triangle as usize * 3..][..3];
        let w = hit.barycentrics;
        let normal = mesh.normals[corners[0] as usize] * w.x
            + mesh.normals[corners[1] as usize] * w.y
            + mesh.normals[corners[2] as usize] * w.z;
        normal.try_normalize().unwrap_or(face)
    }

    /// Whether a hit on `occluder` with barycentrics `w` is on an opaque part of its texture.
    fn blocks(&self, occluder: &OccluderTriangle, w: Vec3) -> bool {
        let mesh = self.meshes[occluder.mesh as usize];
        let (Some(alpha), Some(uv0)) = (&mesh.alpha, &mesh.uv0) else {
            return true;
        };
        let corners = &mesh.indices[occluder.triangle as usize * 3..][..3];
        let uv: Vec2 = uv0[corners[0] as usize] * w.x
            + uv0[corners[1] as usize] * w.y
            + uv0[corners[2] as usize] * w.z;
        alpha.is_opaque(uv)
    }

    fn hit_distance(&self, index: u32, ray: &Ray, max: f32, alpha_test: bool) -> f32 {
        let occluder = &self.triangles[index as usize];
        match intersect(&occluder.vertices, ray.origin, ray.direction, max) {
            Some((t, w)) if !alpha_test || self.blocks(occluder, w) => t,
            _ => f32::INFINITY,
        }
    }
}

/// A ray whose `t < tmax` test (used by obvhs) means `t ≤ max_distance`. Adding zero turns
/// −0 components into +0: obvhs takes a negative inverse for −0 but tests `direction < 0` to
/// pick the slab planes, so such rays would miss everything.
fn segment(origin: Vec3, direction: Vec3, max_distance: f32) -> Ray {
    Ray::new(
        origin.into(),
        (direction + Vec3::ZERO).into(),
        0.0,
        max_distance.max(0.0).next_up(),
    )
}

impl RayCaster for Occluders<'_> {
    fn occluded(&self, origin: Vec3, direction: Vec3, max_distance: f32, alpha_test: bool) -> bool {
        let Some(bvh) = &self.bvh else {
            return false;
        };
        let ray = segment(origin, direction, max_distance);
        !bvh.ray_traverse_miss(ray, |ray, id| {
            self.hit_distance(bvh.primitive_indices[id], ray, max_distance, alpha_test)
        })
    }

    fn closest_hit(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        alpha_test: bool,
    ) -> Option<SurfaceHit> {
        let bvh = self.bvh.as_ref()?;
        let ray = segment(origin, direction, max_distance);
        let mut hit = RayHit::none();
        let mut nearest: Option<(f32, u32)> = None;
        bvh.ray_traverse(ray, &mut hit, |ray, id| {
            let index = bvh.primitive_indices[id];
            let t = self.hit_distance(index, ray, max_distance, alpha_test);
            // Equal distances (shared edges) resolve to the lowest triangle index, so the
            // result doesn't depend on the BVH's traversal order.
            if t.is_finite() && nearest.is_none_or(|(d, i)| t < d || (t == d && index < i)) {
                nearest = Some((t, index));
            }
            t
        });
        let (distance, occluder) = nearest?;
        let vertices = &self.triangles[occluder as usize].vertices;
        let (_, barycentrics) = intersect(vertices, ray.origin, ray.direction, f32::INFINITY)?;
        Some(SurfaceHit {
            distance,
            occluder,
            barycentrics,
        })
    }
}

/// Two-sided Möller–Trumbore. Returns the distance and the vertex weights for hits with
/// `0 ≤ t ≤ max`.
pub fn intersect(
    vertices: &[Vec3A; 3],
    origin: Vec3A,
    direction: Vec3A,
    max: f32,
) -> Option<(f32, Vec3)> {
    let e1 = vertices[1] - vertices[0];
    let e2 = vertices[2] - vertices[0];
    let p = direction.cross(e2);
    let determinant = e1.dot(p);
    if determinant == 0.0 || !determinant.is_finite() {
        return None;
    }
    let inverse = 1.0 / determinant;
    let s = origin - vertices[0];
    let u = s.dot(p) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inverse;
    (t >= 0.0 && t <= max).then(|| (t, Vec3::new(1.0 - u - v, u, v)))
}
