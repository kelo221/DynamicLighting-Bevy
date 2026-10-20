//! What a `DynLitMaterial` reflects: the sky's and the reflection probes' prefiltered cubes, or
//! a cube of its own, through the same binding.

use bevy::asset::AssetEvent;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureViewDescriptor, TextureViewDimension};

use super::material::DynLitMaterial;
use crate::reflection_probes::REFLECTION_CUBES;
use crate::shared::cube::CUBE_FACE_COUNT;

/// What a `DynLitMaterial` reflects under Metallic shading. A material's own cube takes the
/// sky's binding, which keeps the material within WebGPU's 16 textures per shader stage next to
/// the most textures Bevy's view bindings add (no `Hdr`, and depth, normal and motion-vector
/// prepasses).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Reflection {
    /// The reflection probe the surface is in, or outside every probe the sky light's
    /// prefiltered cube, which turns with the sky light (both in [`REFLECTION_CUBES`]). Without
    /// either the ambient color is reflected.
    #[default]
    Sky,
    /// An environment cube in world space, sampled at a mip level by roughness, in place of the
    /// sky's and the probes' reflections. The image is given a cube-array view of its six
    /// layers while it still has its CPU data, so put it on the material before its first frame
    /// on the GPU, or give it that view yourself.
    Cubemap(Handle<Image>),
}

impl Reflection {
    /// The material's own cube, if it has one.
    pub fn cubemap(&self) -> Option<&Handle<Image>> {
        match self {
            Self::Cubemap(cube) => Some(cube),
            Self::Sky => None,
        }
    }
}

/// The handle [`Reflection::Sky`] binds.
static SKY_CUBE: Handle<Image> = REFLECTION_CUBES;

/// The cube bound for the reflection, as `AsBindGroup` reads texture fields.
impl<'a> From<&'a Reflection> for Option<&'a Handle<Image>> {
    fn from(reflection: &'a Reflection) -> Self {
        Some(reflection.cubemap().unwrap_or(&SKY_CUBE))
    }
}

/// Gives the materials' own reflection cubes a cube-array view, which their binding needs, when
/// the image or the material appears.
pub(super) fn give_cubes_array_views(
    mut material_events: MessageReader<AssetEvent<DynLitMaterial>>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    materials: Res<Assets<DynLitMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut cubes: Local<HashSet<AssetId<Image>>>,
    mut warned: Local<bool>,
) {
    let mut check = Vec::new();
    for event in material_events.read() {
        if let AssetEvent::Added { id } | AssetEvent::Modified { id } = event
            && let Some(cube) = materials.get(*id).and_then(|m| m.reflection.cubemap())
            && cubes.insert(cube.id())
        {
            check.push(cube.id());
        }
    }
    for event in image_events.read() {
        if let AssetEvent::Added { id } | AssetEvent::Modified { id } = event
            && cubes.contains(id)
        {
            check.push(*id);
        }
    }
    for id in check {
        let Some(image) = images.get(id) else {
            continue;
        };
        let view = image
            .texture_view_descriptor
            .as_ref()
            .and_then(|view| view.dimension);
        if view == Some(TextureViewDimension::CubeArray) {
            continue;
        }
        let layers = image.texture_descriptor.size.depth_or_array_layers;
        if image.data.is_none() || layers % CUBE_FACE_COUNT != 0 {
            if !*warned {
                *warned = true;
                warn!(
                    "dynlight: a material's reflection cube needs six layers and a cube-array                      view; give the image `TextureViewDimension::CubeArray`"
                );
            }
            continue;
        }
        if let Some(mut image) = images.get_mut(id) {
            image.texture_view_descriptor = Some(TextureViewDescriptor {
                dimension: Some(TextureViewDimension::CubeArray),
                ..default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(reflection: &Reflection) -> Option<Handle<Image>> {
        let handle: Option<&Handle<Image>> = reflection.into();
        handle.cloned()
    }

    #[test]
    fn a_reflection_cube_takes_the_skys_binding() {
        assert_eq!(bound(&Reflection::Sky), Some(REFLECTION_CUBES));
        let cube = Handle::<Image>::default();
        let own = Reflection::Cubemap(cube.clone());
        assert_eq!(bound(&own), Some(cube));
    }
}
