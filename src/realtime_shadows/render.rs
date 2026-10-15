//! The face pass, after each face camera's depth prepass: depth becomes variance moments with a
//! horizontal blur into a scratch texture, then a vertical blur writes the face's layer of the
//! cube array (Henry's two Gaussian blur blits).

use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::prelude::*;
use bevy::render::extract_component::{ComponentUniforms, DynamicUniformIndex};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{texture_2d, texture_depth_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, Extent3d, FragmentState, Operations, PipelineCache,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipelineDescriptor, ShaderStages,
    TextureDescriptor, TextureDimension, TextureFormat, TextureId, TextureSampleType,
    TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::{ViewUniform, ViewUniformOffset, ViewUniforms};

use super::cameras::ShadowFaceUniform;
use super::cubemaps::{SHADOW_CUBEMAP_FORMATS, SHADOW_CUBEMAPS};
use super::faces::SHADOW_CUBEMAP_RESOLUTION;

const MOMENTS_SHADER: &str = "embedded://dynlight/realtime_shadows/shaders/shadow_moments.wesl";
const BLUR_SHADER: &str = "embedded://dynlight/realtime_shadows/shaders/shadow_blur.wesl";

#[derive(Resource)]
pub(super) struct ShadowPipelines {
    moments_layout: BindGroupLayoutDescriptor,
    blur_layout: BindGroupLayoutDescriptor,
    /// One per format the cube array can have.
    variants: Vec<FormatVariant>,
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
        &BindGroupLayoutEntries::single(
            ShaderStages::FRAGMENT,
            texture_2d(TextureSampleType::Float { filterable: false }),
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
    let scratch = |format: TextureFormat| {
        device
            .create_texture(&TextureDescriptor {
                label: Some("dynlight_shadow_scratch"),
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
            scratch: scratch(format),
        })
        .collect();
    commands.insert_resource(ShadowPipelines {
        moments_layout,
        blur_layout,
        variants,
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

type FaceView = (
    &'static ShadowFaceUniform,
    &'static DynamicUniformIndex<ShadowFaceUniform>,
    &'static ViewPrepassTextures,
    &'static ViewUniformOffset,
);

#[allow(clippy::too_many_arguments)]
pub(super) fn shadow_face_pass(
    view: ViewQuery<FaceView>,
    pipelines: Res<ShadowPipelines>,
    layers: Res<ShadowLayerViews>,
    cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    faces: Res<ComponentUniforms<ShadowFaceUniform>>,
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (face, face_index, prepass, view_offset) = view.into_inner();
    let Some(variant) = pipelines
        .variants
        .iter()
        .find(|variant| Some(variant.format) == layers.format)
    else {
        return;
    };
    let (Some(moments), Some(blur), Some(depth), Some(target)) = (
        cache.get_render_pipeline(variant.moments),
        cache.get_render_pipeline(variant.blur),
        prepass.depth_only_view(),
        layers.layers.get(face.layer as usize),
    ) else {
        return;
    };
    let (Some(view_binding), Some(face_binding)) =
        (view_uniforms.uniforms.binding(), faces.uniforms().binding())
    else {
        return;
    };
    let moments_group = device.create_bind_group(
        "dynlight_shadow_moments",
        &cache.get_bind_group_layout(&pipelines.moments_layout),
        &BindGroupEntries::sequential((depth, view_binding, face_binding)),
    );
    let blur_group = device.create_bind_group(
        "dynlight_shadow_blur",
        &cache.get_bind_group_layout(&pipelines.blur_layout),
        &BindGroupEntries::single(&variant.scratch),
    );

    let encoder = ctx.command_encoder();
    let passes = [
        (moments, &moments_group, &variant.scratch, true),
        (blur, &blur_group, target, false),
    ];
    for (pipeline, group, target, needs_offsets) in passes {
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
        if needs_offsets {
            pass.set_bind_group(0, group, &[view_offset.offset, face_index.index()]);
        } else {
            pass.set_bind_group(0, group, &[]);
        }
        pass.draw(0..3, 0..1);
    }
}
