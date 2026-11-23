//! Plain data of a bake's prepared GPU buffers, stored in [`DynLightBake`](super::DynLightBake).
//! `baked_lighting` builds and reads them (`layout.rs` holds their impls).

use std::sync::Arc;

use bevy::platform::collections::HashMap;

use crate::shared::BakeId;

/// Where a baked mesh's data went, for tagging its entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receiver {
    /// The `MeshTag` value that selects this record.
    pub tag: u32,
    pub content_hash: u64,
    pub triangle_count: u32,
    pub occludes: bool,
}

/// Contents of both buffers plus the receiver lookup. The words are shared, so the asset that
/// prepared them, [`AppliedBake`](super::AppliedBake) and the upload all see one copy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedBuffers {
    pub triangles: Arc<Vec<u32>>,
    pub aux: Arc<Vec<u32>>,
    pub receivers: HashMap<BakeId, Receiver>,
}
