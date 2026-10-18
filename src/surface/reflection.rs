//! What a `DynLitMaterial` reflects: the sky's prefiltered cube or one of its own, through the
//! same binding.

use bevy::prelude::*;

use crate::environment::SKY_SPECULAR;

/// What a `DynLitMaterial` reflects under Metallic shading. A material's own cube takes the
/// sky's binding, which keeps the material within WebGPU's 16 textures per shader stage next to
/// the most textures Bevy's view bindings add (no `Hdr`, and depth, normal and motion-vector
/// prepasses).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Reflection {
    /// The sky light's prefiltered cube ([`SKY_SPECULAR`]), which turns with the sky light.
    /// Without a sky light the ambient color is reflected.
    #[default]
    Sky,
    /// An environment cube in world space, sampled at a mip level by roughness, in place of the
    /// sky's reflection.
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
static SKY_CUBE: Handle<Image> = SKY_SPECULAR;

/// The cube bound for the reflection, as `AsBindGroup` reads texture fields.
impl<'a> From<&'a Reflection> for Option<&'a Handle<Image>> {
    fn from(reflection: &'a Reflection) -> Self {
        Some(reflection.cubemap().unwrap_or(&SKY_CUBE))
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
        assert_eq!(bound(&Reflection::Sky), Some(SKY_SPECULAR));
        let cube = Handle::<Image>::default();
        let own = Reflection::Cubemap(cube.clone());
        assert_eq!(bound(&own), Some(cube));
    }
}
