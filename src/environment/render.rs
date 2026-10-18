//! The background pass in the render world: after the opaque pass, a fullscreen triangle at the
//! far plane fills every pixel no surface covered with the sky, before transparent surfaces
//! blend over it. Cameras without `Hdr` tonemap the sky in its shader, as the surfaces do.

use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::core_pipeline::tonemapping::{
    Tonemapping, TonemappingLuts, get_lut_bind_group_layout_entries, get_lut_bindings,
};
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    AddressMode, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, CompareFunction, DepthStencilState,
    FilterMode, FragmentState, MipmapFilterMode, MultisampleState, PipelineCache,
    RenderPassDescriptor, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderStages, ShaderType, SpecializedRenderPipeline, SpecializedRenderPipelines, StoreOp,
    TextureFormat, TextureSampleType, UniformBuffer, VertexState,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::{
    ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset,
    ViewUniforms,
};
use bevy::shader::ShaderDefVal;

use super::{NoSkyBackground, SKY_BACKGROUND, SkyBackground};

const SHADER: &str = "embedded://dynlight/environment/shaders/background.wesl";

/// Mirrors `Background` in `background.wesl`.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub(super) struct BackgroundUniform {
    right: Vec4,
    up: Vec4,
    back: Vec4,
    tint: Vec4,
}

#[derive(Resource)]
pub(super) struct BackgroundPipeline {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    shader: Handle<Shader>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct BackgroundPipelineKey {
    target_format: TextureFormat,
    samples: u32,
    /// The tonemapping the shader applies, for cameras without `Hdr`.
    tonemapping: Option<Tonemapping>,
}

/// Where the tonemapping lookup table is bound.
const LUT_TEXTURE_BINDING: u32 = 4;
const LUT_SAMPLER_BINDING: u32 = 5;

/// The shader defs of Bevy's in-shader tonemapping with `tonemapping`, as its mesh pipeline
/// sets them for cameras without `Hdr`.
fn tonemap_defs(tonemapping: Tonemapping) -> Vec<ShaderDefVal> {
    let method = match tonemapping {
        Tonemapping::None | Tonemapping::Linear => "TONEMAP_METHOD_LINEAR",
        Tonemapping::Reinhard => "TONEMAP_METHOD_REINHARD",
        Tonemapping::ReinhardLuminance => "TONEMAP_METHOD_REINHARD_LUMINANCE",
        Tonemapping::AcesFitted => "TONEMAP_METHOD_ACES_FITTED",
        Tonemapping::AgX => "TONEMAP_METHOD_AGX",
        Tonemapping::SomewhatBoringDisplayTransform => {
            "TONEMAP_METHOD_SOMEWHAT_BORING_DISPLAY_TRANSFORM"
        }
        Tonemapping::TonyMcMapface => "TONEMAP_METHOD_TONY_MC_MAPFACE",
        Tonemapping::BlenderFilmic => "TONEMAP_METHOD_BLENDER_FILMIC",
        Tonemapping::KhronosPbrNeutral => "TONEMAP_METHOD_PBR_NEUTRAL",
    };
    vec![
        "TONEMAP".into(),
        method.into(),
        ShaderDefVal::UInt(
            "TONEMAPPING_LUT_TEXTURE_BINDING_INDEX".into(),
            LUT_TEXTURE_BINDING,
        ),
        ShaderDefVal::UInt(
            "TONEMAPPING_LUT_SAMPLER_BINDING_INDEX".into(),
            LUT_SAMPLER_BINDING,
        ),
    ]
}

impl SpecializedRenderPipeline for BackgroundPipeline {
    type Key = BackgroundPipelineKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("dynlight_sky_background_pipeline".into()),
            layout: vec![self.layout.clone()],
            vertex: VertexState {
                shader: self.shader.clone(),
                entry_point: Some("vertex".into()),
                ..default()
            },
            fragment: Some(FragmentState {
                shader: self.shader.clone(),
                shader_defs: key.tonemapping.map(tonemap_defs).unwrap_or_default(),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: key.target_format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            // Reverse Z: the triangle sits at depth 0, so it only passes where nothing was drawn.
            depth_stencil: Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: default(),
                bias: default(),
            }),
            multisample: MultisampleState {
                count: key.samples,
                ..default()
            },
            ..default()
        }
    }
}

pub(super) fn init_background_pipeline(
    mut commands: Commands,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
) {
    let [lut_texture, lut_sampler] = get_lut_bind_group_layout_entries();
    let layout = BindGroupLayoutDescriptor::new(
        "dynlight_sky_background_bind_group_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<ViewUniform>(true),
                uniform_buffer::<BackgroundUniform>(false),
                lut_texture,
                lut_sampler,
            ),
        ),
    );
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("dynlight_sky_background_sampler"),
        address_mode_u: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..default()
    });
    commands.insert_resource(BackgroundPipeline {
        layout,
        sampler,
        shader: assets.load(SHADER),
    });
    commands.init_resource::<BackgroundUniforms>();
}

#[derive(Resource, Default)]
pub(super) struct BackgroundUniforms(UniformBuffer<BackgroundUniform>);

#[derive(Component)]
pub(super) struct BackgroundPipelineId(CachedRenderPipelineId);

type PipelineView = (
    Entity,
    &'static ExtractedView,
    &'static Msaa,
    Option<&'static ExtractedCamera>,
    Option<&'static Tonemapping>,
);

type PassView = (
    &'static ExtractedCamera,
    &'static ViewTarget,
    &'static ViewDepthStencilTexture,
    &'static ViewUniformOffset,
    &'static BackgroundPipelineId,
    Option<&'static Tonemapping>,
);

pub(super) fn prepare_background_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    pipeline: Res<BackgroundPipeline>,
    mut pipelines: ResMut<SpecializedRenderPipelines<BackgroundPipeline>>,
    views: Query<PipelineView, Without<NoSkyBackground>>,
) {
    for (entity, view, msaa, camera, tonemapping) in &views {
        let hdr = camera.is_some_and(|camera| camera.hdr);
        let key = BackgroundPipelineKey {
            target_format: view.target_format,
            samples: msaa.samples(),
            tonemapping: tonemapping.copied().filter(|t| !hdr && t.is_enabled()),
        };
        let id = pipelines.specialize(&cache, &pipeline, key);
        commands.entity(entity).insert(BackgroundPipelineId(id));
    }
}

pub(super) fn prepare_background_uniform(
    sky: Res<SkyBackground>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut uniforms: ResMut<BackgroundUniforms>,
) {
    let Some(frame) = sky.0 else {
        return;
    };
    uniforms.0.set(BackgroundUniform {
        right: frame.right.extend(0.0),
        up: frame.up.extend(0.0),
        back: frame.back.extend(0.0),
        tint: frame.tint.extend(1.0),
    });
    uniforms.0.write_buffer(&device, &queue);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn background_pass(
    view: ViewQuery<PassView, Without<NoSkyBackground>>,
    sky: Res<SkyBackground>,
    pipeline: Res<BackgroundPipeline>,
    cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    uniforms: Res<BackgroundUniforms>,
    images: Res<RenderAssets<GpuImage>>,
    (luts, fallback): (Res<TonemappingLuts>, Res<FallbackImage>),
    device: Res<RenderDevice>,
    mut ctx: RenderContext,
) {
    let (camera, target, depth, view_offset, pipeline_id, tonemapping) = view.into_inner();
    if sky.0.is_none() {
        return;
    }
    let (Some(render_pipeline), Some(image), Some(view_binding), Some(background)) = (
        cache.get_render_pipeline(pipeline_id.0),
        images.get(&SKY_BACKGROUND),
        view_uniforms.uniforms.binding(),
        uniforms.0.binding(),
    ) else {
        return;
    };
    let tonemapping = tonemapping.copied().unwrap_or_default();
    let (lut, lut_sampler) = get_lut_bindings(&images, &luts, &tonemapping, &fallback);
    let bind_group = device.create_bind_group(
        "dynlight_sky_background_bind_group",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            &image.texture_view,
            &pipeline.sampler,
            view_binding,
            background,
            lut,
            lut_sampler,
        )),
    );
    let color_attachments = [Some(target.get_color_attachment())];
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("dynlight_sky_background_pass"),
            color_attachments: &color_attachments,
            depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    if let Some(viewport) = &camera.viewport {
        let (position, size) = (viewport.physical_position, viewport.physical_size);
        pass.set_viewport(
            position.x as f32,
            position.y as f32,
            size.x as f32,
            size.y as f32,
            viewport.depth.start,
            viewport.depth.end,
        );
    }
    pass.set_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[view_offset.offset]);
    pass.draw(0..3, 0..1);
}
