//! The render phase of shadow cube faces: casters binned by pipeline, material bind group and
//! mesh slabs, then by mesh, like Bevy's own shadow phase. Drawn with Bevy's prepass draw
//! commands into the face pass's depth texture.

use core::marker::PhantomData;
use core::ops::Range;

use bevy::asset::UntypedAssetId;
use bevy::ecs::query::ROQueryItem;
use bevy::ecs::system::SystemParamItem;
use bevy::ecs::system::lifetimeless::{Read, SRes};
use bevy::prelude::*;
use bevy::render::mesh::allocator::MeshSlabs;
use bevy::render::render_phase::{
    BinnedPhaseItem, CachedRenderPipelinePhaseItem, DrawFunctionId, PhaseItem,
    PhaseItemBatchSetKey, PhaseItemExtraIndex, RenderCommand, RenderCommandResult,
    TrackedRenderPass,
};
use bevy::render::render_resource::CachedRenderPipelineId;
use bevy::render::sync_world::MainEntity;

use super::extract::ShadowFaceView;
use super::tracking::ShadowFeedback;

/// Draws a caster with Bevy's command `C`. A draw that doesn't happen (pipelines, mesh data or
/// bind groups not ready yet, Bevy's mesh preprocessing still compiling) reports the face, so it
/// renders again next frame.
pub(super) struct Reported<C>(PhantomData<C>);

impl<P: PhaseItem, C: RenderCommand<P>> RenderCommand<P> for Reported<C> {
    type Param = (C::Param, SRes<ShadowFeedback>);
    type ViewQuery = (C::ViewQuery, Read<ShadowFaceView>);
    type ItemQuery = C::ItemQuery;

    fn render<'w>(
        item: &P,
        (view, face): ROQueryItem<'w, '_, Self::ViewQuery>,
        entity: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        (param, feedback): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let result = C::render(item, view, entity, param, pass);
        if !matches!(result, RenderCommandResult::Success) {
            feedback.report_layer(face.layer);
        }
        result
    }
}

/// One batch of casters in a shadow cube face.
pub(super) struct ShadowFace3d {
    batch_set_key: FaceBatchSetKey,
    representative_entity: (Entity, MainEntity),
    batch_range: Range<u32>,
    extra_index: PhaseItemExtraIndex,
}

/// What casters drawn together share.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct FaceBatchSetKey {
    pub pipeline: CachedRenderPipelineId,
    pub draw_function: DrawFunctionId,
    /// The material's bind group; `None` for depth-only casters, which bind no material.
    pub material_bind_group_index: Option<u32>,
    pub slabs: MeshSlabs,
}

impl PhaseItemBatchSetKey for FaceBatchSetKey {
    fn indexed(&self) -> bool {
        self.slabs.index_slab_id.is_some()
    }
}

/// Casters of the same mesh go into one bin.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct FaceBinKey {
    pub asset_id: UntypedAssetId,
}

impl PhaseItem for ShadowFace3d {
    #[inline]
    fn entity(&self) -> Entity {
        self.representative_entity.0
    }

    #[inline]
    fn main_entity(&self) -> MainEntity {
        self.representative_entity.1
    }

    #[inline]
    fn draw_function(&self) -> DrawFunctionId {
        self.batch_set_key.draw_function
    }

    #[inline]
    fn batch_range(&self) -> &Range<u32> {
        &self.batch_range
    }

    #[inline]
    fn batch_range_mut(&mut self) -> &mut Range<u32> {
        &mut self.batch_range
    }

    #[inline]
    fn extra_index(&self) -> PhaseItemExtraIndex {
        self.extra_index.clone()
    }

    #[inline]
    fn batch_range_and_extra_index_mut(&mut self) -> (&mut Range<u32>, &mut PhaseItemExtraIndex) {
        (&mut self.batch_range, &mut self.extra_index)
    }
}

impl BinnedPhaseItem for ShadowFace3d {
    type BatchSetKey = FaceBatchSetKey;
    type BinKey = FaceBinKey;

    #[inline]
    fn new(
        batch_set_key: Self::BatchSetKey,
        _bin_key: Self::BinKey,
        representative_entity: (Entity, MainEntity),
        batch_range: Range<u32>,
        extra_index: PhaseItemExtraIndex,
    ) -> Self {
        Self {
            batch_set_key,
            representative_entity,
            batch_range,
            extra_index,
        }
    }
}

impl CachedRenderPipelinePhaseItem for ShadowFace3d {
    #[inline]
    fn cached_pipeline(&self) -> CachedRenderPipelineId {
        self.batch_set_key.pipeline
    }
}
