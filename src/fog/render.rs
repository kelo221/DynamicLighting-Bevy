//! The fog pass in the render world: a fullscreen triangle per [`FogPostProcess`] camera, after
//! tonemapping so fog blends in display space like Henry's.

use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, texture_2d, texture_depth_2d,
    texture_depth_2d_multisampled, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, FragmentState, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderStages, SpecializedRenderPipeline,
    SpecializedRenderPipelines, TextureFormat, TextureSampleType,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::storage::GpuShaderBuffer;
use bevy::render::view::{
    ExtractedView, Msaa, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
};

use super::volumes::{FOG_BUFFER, FogPostProcess};

const SHADER: &str = "embedded://dynlight/fog/shaders/fog.wesl";

#[derive(Resource)]
pub(super) struct FogPipeline {
    layouts: [BindGroupLayoutDescriptor; 2],
    sampler: Sampler,
    shader: Handle<Shader>,
    vertex: FullscreenShader,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct FogPipelineKey {
    target_format: TextureFormat,
    multisampled: bool,
}

impl SpecializedRenderPipeline for FogPipeline {
    type Key = FogPipelineKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        let mut shader_defs = Vec::new();
        if key.multisampled {
            shader_defs.push("MULTISAMPLED".into());
        }
        RenderPipelineDescriptor {
            label: Some("dynlight_fog_pipeline".into()),
            layout: vec![self.layouts[usize::from(key.multisampled)].clone()],
            vertex: self.vertex.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs,
                targets: vec![Some(ColorTargetState {
                    format: key.target_format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        }
    }
}

pub(super) fn init_fog_pipeline(
    mut commands: Commands,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
    vertex: Res<FullscreenShader>,
) {
    let layout = |multisampled: bool| {
        let depth = if multisampled {
            texture_depth_2d_multisampled()
        } else {
            texture_depth_2d()
        };
        BindGroupLayoutDescriptor::new(
            "dynlight_fog_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    depth,
                    uniform_buffer::<ViewUniform>(true),
                    storage_buffer_read_only_sized(false, None),
                ),
            ),
        )
    };
    commands.insert_resource(FogPipeline {
        layouts: [layout(false), layout(true)],
        sampler: device.create_sampler(&SamplerDescriptor::default()),
        shader: assets.load(SHADER),
        vertex: vertex.clone(),
    });
}

#[derive(Component)]
pub(super) struct FogPipelineId(CachedRenderPipelineId);

pub(super) fn prepare_fog_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    pipeline: Res<FogPipeline>,
    mut pipelines: ResMut<SpecializedRenderPipelines<FogPipeline>>,
    views: Query<(Entity, &ExtractedView, &Msaa), With<FogPostProcess>>,
) {
    for (entity, view, msaa) in &views {
        let key = FogPipelineKey {
            target_format: view.target_format,
            multisampled: msaa.samples() > 1,
        };
        let id = pipelines.specialize(&cache, &pipeline, key);
        commands.entity(entity).insert(FogPipelineId(id));
    }
}

pub(super) fn fog_pass(
    view: ViewQuery<
        (
            &ViewTarget,
            &ViewUniformOffset,
            &ViewPrepassTextures,
            &FogPipelineId,
            &Msaa,
        ),
        With<FogPostProcess>,
    >,
    pipeline: Res<FogPipeline>,
    cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (target, view_offset, prepass, pipeline_id, msaa) = view.into_inner();
    let (Some(render_pipeline), Some(depth), Some(view_binding), Some(volumes)) = (
        cache.get_render_pipeline(pipeline_id.0),
        prepass.depth_only_view(),
        view_uniforms.uniforms.binding(),
        buffers.get(&FOG_BUFFER),
    ) else {
        return;
    };
    let post_process = target.post_process_write();
    let layout = &pipeline.layouts[usize::from(msaa.samples() > 1)];
    let bind_group = device.create_bind_group(
        "dynlight_fog_bind_group",
        &cache.get_bind_group_layout(layout),
        &BindGroupEntries::sequential((
            post_process.source,
            &pipeline.sampler,
            depth,
            view_binding,
            volumes.buffer.as_entire_binding(),
        )),
    );
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("dynlight_fog_pass"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post_process.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[view_offset.offset]);
    pass.draw(0..3, 0..1);
}
