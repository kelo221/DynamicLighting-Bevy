//! What the shadow views render through: a render target per layer of the cube array and of the
//! sun's cascade array, the depth and scratch textures of the cascades (the cube faces' are in
//! [`ShadowPipelines`]), and the bind groups of both passes.

use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::prelude::*;
use bevy::render::extract_component::ComponentUniforms;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, Extent3d, PipelineCache, TextureDescriptor, TextureDimension,
    TextureFormat, TextureId, TextureUsages, TextureView, TextureViewDescriptor,
    TextureViewDimension,
};
use bevy::render::renderer::RenderDevice;
use bevy::render::texture::GpuImage;
use bevy::render::view::ViewUniforms;

use super::cascades::SUN_CASCADE_RESOLUTION;
use super::cubemaps::{SHADOW_CUBEMAPS, SUN_CASCADES};
use super::extract::ShadowFaceView;
use super::render::ShadowPipelines;
use super::tracking::ShadowTarget;
use super::views::ShadowFaceUniform;

/// A square texture of `size` texels the shadow passes render to and read.
pub(super) fn target_texture(
    device: &RenderDevice,
    label: &'static str,
    size: u32,
    format: TextureFormat,
) -> TextureView {
    device
        .create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width: size,
                height: size,
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
}

/// One render target per layer of an array texture.
#[derive(Default)]
pub(super) struct ArrayLayers {
    texture: Option<TextureId>,
    format: Option<TextureFormat>,
    layers: Vec<TextureView>,
}

impl ArrayLayers {
    /// Follows `image`; returns whether it is a new texture.
    fn update(&mut self, image: Option<&GpuImage>, label: &'static str) -> bool {
        let Some(image) = image else {
            *self = default();
            return false;
        };
        if self.texture == Some(image.texture.id()) {
            return false;
        }
        self.layers = (0..image.texture.depth_or_array_layers())
            .map(|layer| {
                image.texture.create_view(&TextureViewDescriptor {
                    label: Some(label),
                    dimension: Some(TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..default()
                })
            })
            .collect();
        self.texture = Some(image.texture.id());
        self.format = Some(image.texture_descriptor.format);
        true
    }
}

/// The depth and horizontally blurred moments of the cascade being rendered.
struct CascadeScratch {
    depth: TextureView,
    moments: TextureView,
}

/// The render targets of the cube array and the cascade array.
#[derive(Resource, Default)]
pub(super) struct ShadowLayerViews {
    faces: ArrayLayers,
    cascades: ArrayLayers,
    /// Made once the cascade array is full size.
    cascade_scratch: Option<CascadeScratch>,
}

/// What one shadow view renders through.
pub(super) struct ViewTargets<'a> {
    pub depth: &'a TextureView,
    pub scratch: &'a TextureView,
    pub format: TextureFormat,
}

impl ShadowLayerViews {
    /// The layer `target` ends up in.
    pub(super) fn layer(&self, target: ShadowTarget) -> Option<&TextureView> {
        match target {
            ShadowTarget::Face(layer) => self.faces.layers.get(layer as usize),
            ShadowTarget::Cascade(index) => self.cascades.layers.get(index as usize),
        }
    }

    /// The depth and scratch textures `target` renders through, and its format.
    pub(super) fn targets<'a>(
        &'a self,
        target: ShadowTarget,
        pipelines: &'a ShadowPipelines,
    ) -> Option<ViewTargets<'a>> {
        match target {
            ShadowTarget::Face(_) => {
                let format = self.faces.format?;
                Some(ViewTargets {
                    depth: &pipelines.depth,
                    scratch: &pipelines.variant(Some(format))?.scratch,
                    format,
                })
            }
            ShadowTarget::Cascade(_) => {
                let scratch = self.cascade_scratch.as_ref()?;
                Some(ViewTargets {
                    depth: &scratch.depth,
                    scratch: &scratch.moments,
                    format: self.cascades.format?,
                })
            }
        }
    }
}

pub(super) fn prepare_layer_views(
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    mut views: ResMut<ShadowLayerViews>,
) {
    views
        .faces
        .update(images.get(&SHADOW_CUBEMAPS), "dynlight_shadow_face");
    let cascades = images.get(&SUN_CASCADES);
    if !views.cascades.update(cascades, "dynlight_sun_cascade") {
        if cascades.is_none() {
            views.cascade_scratch = None;
        }
        return;
    }
    let full = cascades.is_some_and(|image| image.texture.width() == SUN_CASCADE_RESOLUTION);
    views.cascade_scratch = views.cascades.format.filter(|_| full).map(|format| {
        let size = SUN_CASCADE_RESOLUTION;
        CascadeScratch {
            depth: target_texture(&device, "dynlight_sun_depth", size, CORE_3D_DEPTH_FORMAT),
            moments: target_texture(&device, "dynlight_sun_scratch", size, format),
        }
    });
}

/// The bind groups of the two passes for one kind of view.
pub(super) struct PassGroups {
    pub moments: BindGroup,
    pub blur: BindGroup,
}

/// The bind groups of the face and cascade passes. Every view of a kind binds the same depth
/// texture, scratch texture and uniform buffers, at its own dynamic offsets, so they are made
/// once per frame.
#[derive(Resource, Default)]
pub(super) struct ShadowFaceBindGroups {
    faces: Option<PassGroups>,
    cascades: Option<PassGroups>,
}

impl ShadowFaceBindGroups {
    pub(super) fn get(&self, target: ShadowTarget) -> Option<&PassGroups> {
        match target {
            ShadowTarget::Face(_) => self.faces.as_ref(),
            ShadowTarget::Cascade(_) => self.cascades.as_ref(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_face_bind_groups(
    face_views: Query<&ShadowFaceView>,
    pipelines: Res<ShadowPipelines>,
    layers: Res<ShadowLayerViews>,
    cache: Res<PipelineCache>,
    view_uniforms: Res<ViewUniforms>,
    faces: Res<ComponentUniforms<ShadowFaceUniform>>,
    device: Res<RenderDevice>,
    mut groups: ResMut<ShadowFaceBindGroups>,
) {
    *groups = ShadowFaceBindGroups::default();
    let (Some(view_binding), Some(face_binding)) =
        (view_uniforms.uniforms.binding(), faces.uniforms().binding())
    else {
        return;
    };
    let make = |target: ShadowTarget| {
        let targets = layers.targets(target, &pipelines)?;
        Some(PassGroups {
            moments: device.create_bind_group(
                "dynlight_shadow_moments",
                &cache.get_bind_group_layout(&pipelines.moments_layout),
                &BindGroupEntries::sequential((
                    targets.depth,
                    view_binding.clone(),
                    face_binding.clone(),
                )),
            ),
            blur: device.create_bind_group(
                "dynlight_shadow_blur",
                &cache.get_bind_group_layout(&pipelines.blur_layout),
                &BindGroupEntries::sequential((targets.scratch, face_binding.clone())),
            ),
        })
    };
    let first = |cascade: bool| {
        face_views
            .iter()
            .find(|view| matches!(view.target, ShadowTarget::Cascade(_)) == cascade)
    };
    groups.faces = first(false).and_then(|view| make(view.target));
    groups.cascades = first(true).and_then(|view| make(view.target));
}
