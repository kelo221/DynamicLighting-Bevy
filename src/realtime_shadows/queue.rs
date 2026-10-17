//! Fills the phase of every face rendering this frame with its casters, through each material's
//! own prepass pipeline, as Bevy's depth prepass would draw them. A face whose casters aren't all
//! ready (mesh, material or textures not on the GPU yet, pipeline still compiling) is reported,
//! so it renders again next frame.

use std::sync::Arc;

use bevy::asset::UntypedAssetId;
use bevy::ecs::system::{SystemParam, SystemState};
use bevy::material::key::{ErasedMaterialPipelineKey, ErasedMeshPipelineKey};
use bevy::material::{AlphaMode, MaterialProperties, RenderPhaseType};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{
    DrawDepthOnlyPrepass, DrawPrepass, MeshPipelineKey, PreparedMaterial, PrepassFragmentShader,
    PrepassVertexShader, RenderMaterialInstances, RenderMeshInstances,
};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::batching::gpu_preprocessing::{GpuPreprocessingMode, GpuPreprocessingSupport};
use bevy::render::erased_render_asset::ErasedRenderAssets;
use bevy::render::mesh::RenderMesh;
use bevy::render::mesh::allocator::{MeshAllocator, MeshSlabs};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, ViewBinnedRenderPhases,
};
use bevy::render::render_resource::CachedRenderPipelineId;
use bevy::render::sync_world::MainEntity;
use bevy::render::view::{ExtractedView, RetainedViewEntity};

use super::extract::ShadowFaceView;
use super::phase::{FaceBatchSetKey, FaceBinKey, Reported, ShadowFace3d};
use super::tracking::ShadowFeedback;

#[derive(SystemParam)]
pub(super) struct FaceQueue<'w, 's> {
    views: Query<'w, 's, (&'static ExtractedView, &'static ShadowFaceView)>,
    meshes: Res<'w, RenderAssets<RenderMesh>>,
    mesh_instances: Res<'w, RenderMeshInstances>,
    allocator: Res<'w, MeshAllocator>,
    materials: Res<'w, ErasedRenderAssets<PreparedMaterial>>,
    material_instances: Res<'w, RenderMaterialInstances>,
    phases: ResMut<'w, ViewBinnedRenderPhases<ShadowFace3d>>,
    draw_functions: Res<'w, DrawFunctions<ShadowFace3d>>,
    preprocessing: Res<'w, GpuPreprocessingSupport>,
    feedback: Res<'w, ShadowFeedback>,
}

/// One caster of one face, waiting for its pipeline.
struct Queued {
    entity: (Entity, MainEntity),
    pipeline: usize,
    depth_only: bool,
    material_bind_group: u32,
    mesh: UntypedAssetId,
    slabs: MeshSlabs,
    uniform: InputUniformIndex,
    batch: bool,
}

struct Face {
    view: RetainedViewEntity,
    layer: u32,
    casters: Vec<Queued>,
    missed: bool,
}

type PipelineKey = (ErasedMaterialPipelineKey, MeshVertexBufferLayoutRef);

/// Bevy's prepass key for a caster with `properties` and mesh key bits `mesh_bits`, as a
/// single-sampled depth prepass view sees it; `None` for materials that don't cast.
fn caster_key(properties: &MaterialProperties, mesh_bits: u64) -> Option<MeshPipelineKey> {
    if properties.reads_view_transmission_texture
        || !matches!(
            properties.render_phase_type,
            RenderPhaseType::Opaque | RenderPhaseType::AlphaMask
        )
    {
        return None;
    }
    let mut key = MeshPipelineKey::from_msaa_samples(1)
        | MeshPipelineKey::DEPTH_PREPASS
        | MeshPipelineKey::from_bits_retain(mesh_bits);
    match properties.alpha_mode {
        AlphaMode::Opaque => {}
        AlphaMode::Mask(_) | AlphaMode::AlphaToCoverage => key |= MeshPipelineKey::MAY_DISCARD,
        _ => return None,
    }
    if properties.get_shader(PrepassVertexShader).is_some()
        || properties.get_shader(PrepassFragmentShader).is_some()
    {
        key |= MeshPipelineKey::PREPASS_READS_MATERIAL;
    }
    Some(key)
}

/// Specializes and queues the casters of every rendering face; each gets a fresh phase.
pub(super) fn queue_shadow_faces(world: &mut World, state: &mut SystemState<FaceQueue>) {
    let mut faces = Vec::new();
    let mut keys: Vec<(PipelineKey, Arc<MaterialProperties>)> = Vec::new();
    {
        let Ok(queue) = state.get_mut(world) else {
            return;
        };
        let mut indices: HashMap<PipelineKey, usize> = HashMap::default();
        for (view, face_view) in &queue.views {
            let mut face = Face {
                view: view.retained_view_entity,
                layer: face_view.layer,
                casters: Vec::new(),
                missed: false,
            };
            for &(entity, main_entity) in &face_view.casters {
                let material = queue.material_instances.instances.get(&main_entity);
                let instance = queue.mesh_instances.render_mesh_queue_data(main_entity);
                let (Some(material), Some(instance)) = (material, instance) else {
                    face.missed = true;
                    continue;
                };
                let asset_id = material.asset_id;
                let prepared = queue.materials.get(asset_id);
                let mesh_id = instance.mesh_asset_id();
                let mesh = queue.meshes.get(mesh_id);
                let slabs = queue.allocator.mesh_slabs(&mesh_id);
                let (Some(prepared), Some(mesh), Some(slabs)) = (prepared, mesh, slabs) else {
                    face.missed = true;
                    continue;
                };
                let properties = &prepared.properties;
                if properties.prepass_specialize.is_none() || !properties.prepass_enabled {
                    continue;
                }
                let Some(mesh_key) = caster_key(properties, mesh.key_bits.bits()) else {
                    continue;
                };
                let key = ErasedMaterialPipelineKey {
                    type_id: asset_id.type_id(),
                    mesh_key: ErasedMeshPipelineKey::new(mesh_key),
                    material_key: properties.material_key.clone(),
                };
                let pipeline = *indices
                    .entry((key, mesh.layout.clone()))
                    .or_insert_with_key(|key| {
                        keys.push((key.clone(), properties.clone()));
                        keys.len() - 1
                    });
                let reads_material =
                    MeshPipelineKey::MAY_DISCARD | MeshPipelineKey::PREPASS_READS_MATERIAL;
                face.casters.push(Queued {
                    entity: (entity, main_entity),
                    pipeline,
                    depth_only: !mesh_key.intersects(reads_material),
                    material_bind_group: prepared.binding.group.0,
                    mesh: mesh_id.into(),
                    slabs,
                    uniform: instance.current_uniform_index,
                    batch: instance.should_batch(),
                });
            }
            faces.push(face);
        }
    }

    let pipelines: Vec<Option<CachedRenderPipelineId>> = keys
        .iter()
        .map(|((key, layout), properties)| {
            let specialize = properties.prepass_specialize?;
            specialize(world, key, layout, properties)
                .inspect_err(|error| error!("dynlight shadows: {error}"))
                .ok()
        })
        .collect();

    let Ok(mut queue) = state.get_mut(world) else {
        return;
    };
    let draw_functions = queue.draw_functions.read();
    let (draw, draw_depth_only) = (
        draw_functions.id::<Reported<DrawPrepass>>(),
        draw_functions.id::<Reported<DrawDepthOnlyPrepass>>(),
    );
    drop(draw_functions);
    let mode = queue
        .preprocessing
        .min(GpuPreprocessingMode::PreprocessingOnly);
    let live: HashSet<RetainedViewEntity> = faces.iter().map(|face| face.view).collect();
    queue.phases.retain(|view, _| live.contains(view));
    for face in faces {
        queue.phases.remove(&face.view);
        queue.phases.prepare_for_new_frame(face.view, mode);
        let Some(phase) = queue.phases.get_mut(&face.view) else {
            continue;
        };
        // Casters whose pipeline failed to specialize never draw; pipelines still compiling are
        // reported by the draw itself (`Reported`).
        for caster in face.casters {
            let Some(pipeline) = pipelines[caster.pipeline] else {
                continue;
            };
            let batch_set_key = FaceBatchSetKey {
                pipeline,
                draw_function: if caster.depth_only {
                    draw_depth_only
                } else {
                    draw
                },
                material_bind_group_index: (!caster.depth_only)
                    .then_some(caster.material_bind_group),
                slabs: caster.slabs,
            };
            phase.add(
                batch_set_key,
                FaceBinKey {
                    asset_id: caster.mesh,
                },
                caster.entity,
                caster.uniform,
                BinnedRenderPhaseType::mesh(caster.batch, &queue.preprocessing),
            );
        }
        if face.missed {
            queue.feedback.report_layer(face.layer);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_and_masked_materials_cast_and_blended_ones_dont() {
        let mut properties = MaterialProperties::default();
        let opaque = caster_key(&properties, 0).expect("opaque casts");
        assert!(opaque.contains(MeshPipelineKey::DEPTH_PREPASS));
        assert!(!opaque.intersects(MeshPipelineKey::MAY_DISCARD));
        properties.alpha_mode = AlphaMode::Mask(0.5);
        properties.render_phase_type = RenderPhaseType::AlphaMask;
        let masked = caster_key(&properties, 0).expect("masked casts");
        assert!(masked.contains(MeshPipelineKey::MAY_DISCARD));
        properties.alpha_mode = AlphaMode::Blend;
        properties.render_phase_type = RenderPhaseType::Transparent;
        assert_eq!(caster_key(&properties, 0), None);
    }
}
