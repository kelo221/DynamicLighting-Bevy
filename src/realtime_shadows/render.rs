//! The face pass, run for each face view: its casters are drawn into a depth texture shared by
//! all faces, depth becomes variance moments with a horizontal blur into a scratch texture, then
//! a vertical blur writes the face's layer of the cube array (Henry's two Gaussian blur blits).
//! Both blurs read their width from the face uniform.

use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::ViewBinnedRenderPhases;
use bevy::render::render_resource::binding_types::{texture_2d, texture_depth_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, Extent3d, FragmentState, LoadOp,
    Operations, PipelineCache, RenderPassColorAttachment, RenderPassDepthStencilAttachment,
    RenderPassDescriptor, RenderPipelineDescriptor, ShaderStages, StoreOp, TextureDescriptor,
    TextureDimension, TextureFormat, TextureId, TextureSampleType, TextureUsages, TextureView,
    TextureViewDescriptor, TextureViewDimension,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, ViewUniform, ViewUniformOffset, ViewUniforms};

use super::cubemaps::{SHADOW_CUBEMAP_FORMATS, SHADOW_CUBEMAPS};
use super::faces::SHADOW_CUBEMAP_RESOLUTION;
use super::phase::ShadowFace3d;
use super::tracking::ShadowFeedback;
use super::views::ShadowFaceUniform;

const MOMENTS_SHADER: &str = "embedded://dynlight/realtime_shadows/shaders/shadow_moments.wesl";
const BLUR_SHADER: &str = "embedded://dynlight/realtime_shadows/shaders/shadow_blur.wesl";

#[derive(Resource)]
pub(super) struct ShadowPipelines {
    moments_layout: BindGroupLayoutDescriptor,
    blur_layout: BindGroupLayoutDescriptor,
    /// One per format the cube array can have.
    variants: Vec<FormatVariant>,
    /// The depth of the face being rendered; faces render one after another.
    depth: TextureView,
}

impl ShadowPipelines {
    fn variant(&self, format: Option<TextureFormat>) -> Option<&FormatVariant> {
        self.variants
            .iter()
            .find(|variant| Some(variant.format) == format)
    }
}

struct FormatVariant {
    format: TextureFormat,
    moments: CachedRenderPipelineId,
    blur: CachedRenderPipelineId,
    /// Horizontally blurred moments of the face being rendered.
    scratch: TextureView,
}

pub(super) fn init_shadow_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let moments_layout = BindGroupLayoutDescriptor::new(
        "dynlight_shadow_moments_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_depth_2d(),
                uniform_buffer::<ViewUniform>(true),
                uniform_buffer::<ShadowFaceUniform>(true),
            ),
        ),
    );
    let blur_layout = BindGroupLayoutDescriptor::new(
        "dynlight_shadow_blur_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: false }),
                uniform_buffer::<ShadowFaceUniform>(true),
            ),
        ),
    );
    let pipeline = |label: &'static str,
                    layout: &BindGroupLayoutDescriptor,
                    shader: &str,
                    format: TextureFormat| RenderPipelineDescriptor {
        label: Some(label.into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: assets.load(shader.to_owned()),
            targets: vec![Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    };
    let scratch = |label: &'static str, format: TextureFormat| {
        device
            .create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: SHADOW_CUBEMAP_RESOLUTION,
                    height: SHADOW_CUBEMAP_RESOLUTION,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&TextureViewDescriptor::default())
    };
    let variants = SHADOW_CUBEMAP_FORMATS
        .into_iter()
        .filter(|format| {
            let features = format.guaranteed_format_features(device.features());
            features
                .allowed_usages
                .contains(TextureUsages::RENDER_ATTACHMENT)
        })
        .map(|format| FormatVariant {
            format,
            moments: cache.queue_render_pipeline(pipeline(
                "dynlight_shadow_moments",
                &moments_layout,
                MOMENTS_SHADER,
                format,
            )),
            blur: cache.queue_render_pipeline(pipeline(
                "dynlight_shadow_blur",
                &blur_layout,
                BLUR_SHADER,
                format,
            )),
            scratch: scratch("dynlight_shadow_scratch", format),
        })
        .collect();
    commands.insert_resource(ShadowPipelines {
        moments_layout,
        blur_layout,
        variants,
        depth: scratch("dynlight_shadow_depth", CORE_3D_DEPTH_FORMAT),
    });
}

/// One render target per array layer of the current shadow cubemap texture.
#[derive(Resource, Default)]
pub(super) struct ShadowLayerViews {
    texture: Option<TextureId>,
    format: Option<TextureFormat>,
    layers: Vec<TextureView>,
}

pub(super) fn prepare_layer_views(
    images: Res<RenderAssets<GpuImage>>,
    mut views: ResMut<ShadowLayerViews>,
) {
    let Some(image) = images.get(&SHADOW_CUBEMAPS) else {
        *views = ShadowLayerViews::default();
        return;
    };
    if views.texture == Some(image.texture.id()) {
        return;
    }
    views.layers = (0..image.texture.depth_or_array_layers())
        .map(|layer| {
            image.texture.create_view(&TextureViewDescriptor {
                label: Some("dynlight_shadow_face"),
                dimension: Some(TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..default()
            })
        })
        .collect();
    views.texture = Some(image.texture.id());
    views.format = Some(image.texture_descriptor.format);
}

/// The bind groups of the face pass. Every face of a frame binds the same depth texture, scratch
/// texture and uniform buffers, at its own dynamic offsets, so they are made once per frame.
#[derive(Resource, Default)]
pub(super) struct ShadowFaceBindGroups {
    moments: Option<BindGroup>,
    blur: Option<BindGroup>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_face_bind_groups(
    face_views: Query<(), With<ShadowFaceUniform>>,
    pipelines: Res<ShadowPipelines>,
    layers: Res<ShadowLayerViews>,
    cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    faces: Res<ComponentUniforms<ShadowFaceUniform>>,
    device: Res<RenderDevice>,
    mut groups: ResMut<ShadowFaceBindGroups>,
) {
    *groups = ShadowFaceBindGroups::default();
    if face_views.is_empty() {
        return;
    }
    let Some(variant) = pipelines.variant(layers.format) else {
        return;
    };
    let (Some(view_binding), Some(face_binding)) =
        (view_uniforms.uniforms.binding(), faces.uniforms().binding())
    else {
        return;
    };
    groups.moments = Some(device.create_bind_group(
        "dynlight_shadow_moments",
        &cache.get_bind_group_layout(&pipelines.moments_layout),
        &BindGroupEntries::sequential((&pipelines.depth, view_binding, face_binding.clone())),
    ));
    groups.blur = Some(device.create_bind_group(
        "dynlight_shadow_blur",
        &cache.get_bind_group_layout(&pipelines.blur_layout),
        &BindGroupEntries::sequential((&variant.scratch, face_binding)),
    ));
}

type FaceView = (
    Entity,
    &'static ExtractedView,
    &'static ShadowFaceUniform,
    &'static DynamicUniformIndex<ShadowFaceUniform>,
    &'static ViewUniformOffset,
);

/// Renders one face. When it can't, it tells the main world to render the cubes again.
#[allow(clippy::too_many_arguments)]
pub(super) fn shadow_face_pass(
    world: &World,
    view: ViewQuery<FaceView>,
    phases: Res<ViewBinnedRenderPhases<ShadowFace3d>>,
    pipelines: Res<ShadowPipelines>,
    layers: Res<ShadowLayerViews>,
    groups: Res<ShadowFaceBindGroups>,
    cache: Res<PipelineCache>,
    feedback: Res<ShadowFeedback>,
    mut ctx: RenderContext,
) {
    let (view_entity, extracted, face, face_index, view_offset) = view.into_inner();
    let Some(variant) = pipelines.variant(layers.format) else {
        feedback.report_incomplete();
        return;
    };
    let (Some(moments), Some(blur), Some(target), Some(moments_group), Some(blur_group)) = (
        cache.get_render_pipeline(variant.moments),
        cache.get_render_pipeline(variant.blur),
        layers.layers.get(face.layer as usize),
        groups.moments.as_ref(),
        groups.blur.as_ref(),
    ) else {
        feedback.report_incomplete();
        return;
    };
    let depth = &pipelines.depth;

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let time_span = diagnostics.time_span(ctx.command_encoder(), "dynlight_shadow_face");
    {
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("dynlight_shadow_face_depth"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.0),
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(phase) = phases.get(&extracted.retained_view_entity)
            && let Err(error) = phase.render(&mut pass, world, view_entity)
        {
            error!(
                "dynlight shadows: face {} failed to draw: {error:?}",
                face.layer
            );
            feedback.report_layer(face.layer);
        }
    }

    let encoder = ctx.command_encoder();
    let face_offset = face_index.index();
    let moments_offsets = [view_offset.offset, face_offset];
    let passes = [
        (
            moments,
            moments_group,
            &variant.scratch,
            &moments_offsets[..],
        ),
        (blur, blur_group, target, &[face_offset][..]),
    ];
    for (pipeline, group, target, offsets) in passes {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("dynlight_shadow_face"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, offsets);
        pass.draw(0..3, 0..1);
    }
    time_span.end(ctx.command_encoder());
}
