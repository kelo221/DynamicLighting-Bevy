//! The face pass, run for each face and cascade view: its casters are drawn into a depth texture
//! shared by all views of its kind, depth becomes variance moments with a horizontal blur into a
//! scratch texture, then a vertical blur writes the view's layer of the cube or cascade array
//! (Henry's two Gaussian blur blits). Both blurs read their width from the face uniform. A view
//! with no casters only clears its layer.

use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::DynamicUniformIndex;
use bevy::render::render_phase::ViewBinnedRenderPhases;
use bevy::render::render_resource::binding_types::{texture_2d, texture_depth_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroup, BindGroupLayoutDescriptor, BindGroupLayoutEntries, CachedRenderPipelineId,
    ColorTargetState, ColorWrites, CommandEncoder, FragmentState, LoadOp, Operations,
    PipelineCache, RenderPassColorAttachment, RenderPassDepthStencilAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, ShaderStages, StoreOp,
    TextureFormat, TextureSampleType, TextureUsages, TextureView,
};
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::view::{ExtractedView, ViewUniform, ViewUniformOffset};

use super::cubemaps::SHADOW_CUBEMAP_FORMATS;
use super::extract::ShadowFaceView;
use super::faces::SHADOW_CUBEMAP_RESOLUTION;
use super::phase::ShadowFace3d;
use super::targets::{ShadowFaceBindGroups, ShadowLayerViews, target_texture};
use super::tracking::{ShadowFeedback, ShadowTarget};
use super::views::ShadowFaceUniform;

const MOMENTS_SHADER: &str = "embedded://dynlight/realtime_shadows/shaders/shadow_moments.wesl";
const BLUR_SHADER: &str = "embedded://dynlight/realtime_shadows/shaders/shadow_blur.wesl";

#[derive(Resource)]
pub(super) struct ShadowPipelines {
    pub moments_layout: BindGroupLayoutDescriptor,
    pub blur_layout: BindGroupLayoutDescriptor,
    /// One per format the cube array can have.
    variants: Vec<FormatVariant>,
    /// The depth of the face being rendered; faces render one after another.
    pub depth: TextureView,
}

impl ShadowPipelines {
    pub(super) fn variant(&self, format: Option<TextureFormat>) -> Option<&FormatVariant> {
        self.variants
            .iter()
            .find(|variant| Some(variant.format) == format)
    }
}

pub(super) struct FormatVariant {
    format: TextureFormat,
    moments: CachedRenderPipelineId,
    blur: CachedRenderPipelineId,
    /// Horizontally blurred moments of the face being rendered.
    pub scratch: TextureView,
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
        target_texture(&device, label, SHADOW_CUBEMAP_RESOLUTION, format)
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

type FaceView = (
    Entity,
    &'static ExtractedView,
    &'static ShadowFaceView,
    &'static DynamicUniformIndex<ShadowFaceUniform>,
    &'static ViewUniformOffset,
);

/// What a face without casters ends up as: moments (1, 1), the light's radius everywhere.
const UNSHADOWED: LinearRgba = LinearRgba::new(1.0, 1.0, 0.0, 1.0);

/// Renders one face or cascade. When it can't, it tells the main world to render them all
/// again.
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
    let Some(target) = layers.layer(face.target) else {
        feedback.report_incomplete();
        return;
    };
    let span = match face.target {
        ShadowTarget::Face(_) => "dynlight_shadow_face",
        ShadowTarget::Cascade(_) => "dynlight_sun_cascade",
    };
    let phase = phases.get(&extracted.retained_view_entity);
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    // The depth, moments and blur passes of an empty view only turn cleared depth into
    // `UNSHADOWED`, so the layer is cleared to it instead.
    let Some(phase) = phase.filter(|phase| !phase.is_empty()) else {
        let time_span = diagnostics.time_span(ctx.command_encoder(), span);
        color_pass(ctx.command_encoder(), target, UNSHADOWED, None);
        time_span.end(ctx.command_encoder());
        return;
    };
    let through = layers.targets(face.target, &pipelines);
    let variant = through
        .as_ref()
        .and_then(|through| pipelines.variant(Some(through.format)));
    let (Some(through), Some(variant), Some(groups)) = (through, variant, groups.get(face.target))
    else {
        feedback.report_incomplete();
        return;
    };
    let (Some(moments), Some(blur)) = (
        cache.get_render_pipeline(variant.moments),
        cache.get_render_pipeline(variant.blur),
    ) else {
        feedback.report_incomplete();
        return;
    };
    let depth = through.depth;

    let time_span = diagnostics.time_span(ctx.command_encoder(), span);
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
        if let Err(error) = phase.render(&mut pass, world, view_entity) {
            error!(
                "dynlight shadows: {:?} failed to draw: {error:?}",
                face.target
            );
            feedback.report(face.target);
        }
    }

    let encoder = ctx.command_encoder();
    let face_offset = face_index.index();
    let moments_offsets = [view_offset.offset, face_offset];
    let passes = [
        (
            moments,
            &groups.moments,
            through.scratch,
            &moments_offsets[..],
        ),
        (blur, &groups.blur, target, &[face_offset][..]),
    ];
    for (pipeline, group, target, offsets) in passes {
        color_pass(
            encoder,
            target,
            LinearRgba::NONE,
            Some((pipeline, group, offsets)),
        );
    }
    time_span.end(ctx.command_encoder());
}

/// Clears `target` to `clear`, then draws a fullscreen triangle with `draw` if given.
fn color_pass(
    encoder: &mut CommandEncoder,
    target: &TextureView,
    clear: LinearRgba,
    draw: Option<(&RenderPipeline, &BindGroup, &[u32])>,
) {
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("dynlight_shadow_face"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Clear(clear.into()),
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    if let Some((pipeline, group, offsets)) = draw {
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, offsets);
        pass.draw(0..3, 0..1);
    }
}
