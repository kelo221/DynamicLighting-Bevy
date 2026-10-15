//! Content hash of the mesh data a bake depends on, used to detect stale bakes.
//!
//! The baker stores this hash per mesh; at runtime the same hash is computed from the loaded
//! mesh asset, and a mismatch means the bake no longer fits the mesh (vertices moved, the index
//! order changed or the lightmap UVs were re-unwrapped). Only the triangles count, not how they
//! share vertices, so de-indexing a mesh (the triangle-ID fallback) keeps its hash.

use bevy::mesh::{Mesh, VertexAttributeValues};

/// Hashes the local-space position and UV1 (lightmap) coordinate of every triangle corner, in
/// draw order. Values are hashed by their exact bit patterns, so any edit to the triangles
/// changes the hash; vertices no triangle uses don't count.
///
/// The result is identical on every platform and run. Pass the UVs the bake actually used: UV1,
/// or UV0 when the mesh has no UV1.
pub fn mesh_content_hash(positions: &[[f32; 3]], indices: &[u32], uv1: &[[f32; 2]]) -> u64 {
    const MISSING: u32 = u32::MAX;
    let mut hash = WordHash::new();
    hash.write_len(indices.len());
    for &index in indices {
        let index = index as usize;
        match positions.get(index) {
            Some(position) => position
                .iter()
                .for_each(|value| hash.write(value.to_bits())),
            None => hash.write(MISSING),
        }
        match uv1.get(index) {
            Some(uv) => uv.iter().for_each(|value| hash.write(value.to_bits())),
            None => hash.write(MISSING),
        }
    }
    hash.finish()
}

/// [`mesh_content_hash`] of a mesh asset, with the inputs the bake reads: positions, the draw
/// order of the triangles (sequential for non-indexed meshes) and UV1, or UV0 without UV1.
/// `None` for meshes the bake can't use (no positions or no UVs, or vertex data that only
/// exists in the render world).
pub fn hash_mesh(mesh: &Mesh) -> Option<u64> {
    let attribute = |id| mesh.try_attribute_option(id).ok().flatten();
    let Some(VertexAttributeValues::Float32x3(positions)) = attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return None;
    };
    let uvs = attribute(Mesh::ATTRIBUTE_UV_1).or_else(|| attribute(Mesh::ATTRIBUTE_UV_0));
    let Some(VertexAttributeValues::Float32x2(uvs)) = uvs else {
        return None;
    };
    let indices: Vec<u32> = match mesh.try_indices_option().ok()? {
        Some(indices) => indices.iter().map(|index| index as u32).collect(),
        None => (0..positions.len() as u32).collect(),
    };
    Some(mesh_content_hash(positions, &indices, uvs))
}

/// A word-at-a-time multiplicative hash with a SplitMix64 finalizer: fast on large meshes and
/// fully specified here, so the bake and the runtime always agree.
struct WordHash(u64);

impl WordHash {
    const SEED: u64 = 0x6479_6e6c_6967_6874; // "dynlight"
    const MULTIPLIER: u64 = 0x517c_c1b7_2722_0a95;

    fn new() -> Self {
        Self(Self::SEED)
    }

    fn write(&mut self, word: u32) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(word)).wrapping_mul(Self::MULTIPLIER);
    }

    fn write_len(&mut self, len: usize) {
        let len = len as u64;
        self.write(len as u32);
        self.write((len >> 32) as u32);
    }

    fn finish(&self) -> u64 {
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSITIONS: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    const UVS: [[f32; 2]; 3] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
    const PINNED: u64 = 11_458_312_689_423_647_656;

    #[test]
    fn hash_is_stable() {
        let a = mesh_content_hash(&POSITIONS, &[0, 1, 2], &UVS);
        let b = mesh_content_hash(&POSITIONS, &[0, 1, 2], &UVS);
        assert_eq!(a, b);
        // Pinned so an accidental change to the algorithm (which would mark every existing
        // bake as stale) fails loudly.
        assert_eq!(a, PINNED);
    }

    #[test]
    fn every_input_changes_the_hash() {
        let base = mesh_content_hash(&POSITIONS, &[0, 1, 2], &UVS);
        let mut moved = POSITIONS;
        moved[1][0] = 1.0001;
        assert_ne!(base, mesh_content_hash(&moved, &[0, 1, 2], &UVS));
        assert_ne!(base, mesh_content_hash(&POSITIONS, &[0, 2, 1], &UVS));
        let mut unwrapped = UVS;
        unwrapped[2][1] = 0.5;
        assert_ne!(base, mesh_content_hash(&POSITIONS, &[0, 1, 2], &unwrapped));
    }

    #[test]
    fn only_the_triangles_count() {
        let base = mesh_content_hash(&POSITIONS, &[0, 1, 2], &UVS);
        let mut unused = POSITIONS.to_vec();
        unused.push([5.0, 5.0, 5.0]);
        let mut unused_uvs = UVS.to_vec();
        unused_uvs.push([0.5, 0.5]);
        assert_eq!(base, mesh_content_hash(&unused, &[0, 1, 2], &unused_uvs));
        let shared = mesh_content_hash(&POSITIONS, &[0, 1, 2, 2, 1, 0], &UVS);
        let doubled: Vec<_> = [0, 1, 2, 2, 1, 0].map(|i| POSITIONS[i]).to_vec();
        let doubled_uvs: Vec<_> = [0, 1, 2, 2, 1, 0].map(|i| UVS[i]).to_vec();
        assert_eq!(
            shared,
            mesh_content_hash(&doubled, &[0, 1, 2, 3, 4, 5], &doubled_uvs)
        );
    }

    #[test]
    fn missing_vertices_are_hashed_apart() {
        let a = mesh_content_hash(&POSITIONS, &[0, 1, 3], &UVS);
        let b = mesh_content_hash(&POSITIONS, &[0, 1, 2], &UVS);
        assert_ne!(a, b);
        assert_ne!(
            mesh_content_hash(&[], &[], &[]),
            mesh_content_hash(&[], &[0], &[])
        );
    }
}
