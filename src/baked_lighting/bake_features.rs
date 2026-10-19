//! The lit-shader features the applied bake needs: bounce light and the light BVH.

use bevy::prelude::*;

use super::layout::{flags, header};
use super::systems::AppliedBake;
use crate::shared::{ShaderFeatures, ShaderFeaturesInUse};

/// The features a bake's aux buffer needs: `bounce` with the bounce layout, `light_bvh` with a
/// light BVH. The lights and meshes decide the others.
pub fn bake_features(aux: &[u32]) -> ShaderFeatures {
    let word = |index: usize| aux.get(index).copied().unwrap_or(0);
    ShaderFeatures {
        bounce: word(header::FLAGS) & flags::BOUNCE_LAYOUT != 0,
        light_bvh: word(header::BVH_NODE_COUNT) > 0,
        ..ShaderFeatures::NONE
    }
}

/// Marks the applied bake's features as used. Runs every frame, because the use is cleared in
/// `Last`.
pub(super) fn note_bake_features(
    applied: Res<AppliedBake>,
    mut in_use: ResMut<ShaderFeaturesInUse>,
) {
    let used = bake_features(&applied.buffers.aux);
    in_use.0.bounce |= used.bounce;
    in_use.0.light_bvh |= used.light_bvh;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::baked_lighting::BakedBuffers;
    use crate::shared::{
        DynamicLightingSettings, FeatureSelection, LitShaderFeatures, SharedPlugin,
    };

    /// An empty bake's aux buffer with `words` set in its header.
    fn aux(words: &[(usize, u32)]) -> Vec<u32> {
        let mut aux = BakedBuffers::empty().aux;
        for &(index, value) in words {
            aux[index] = value;
        }
        aux
    }

    #[test]
    fn an_empty_bake_needs_nothing() {
        assert_eq!(
            bake_features(&BakedBuffers::empty().aux),
            ShaderFeatures::NONE
        );
        assert_eq!(bake_features(&[]), ShaderFeatures::NONE);
    }

    #[test]
    fn a_light_bvh_needs_the_light_bvh() {
        let used = bake_features(&aux(&[
            (header::BVH_NODE_COUNT, 3),
            (header::CUBE_COUNT, 2),
        ]));
        assert_eq!(
            used,
            ShaderFeatures {
                light_bvh: true,
                ..ShaderFeatures::NONE
            }
        );
    }

    #[test]
    fn suns_and_skies_alone_need_no_light_bvh() {
        let header = [
            (header::SUN_MAP_COUNT, 1),
            (header::SKY_PROBES, 40),
            (header::BAKED_LIGHTS, 2),
        ];
        assert_eq!(bake_features(&aux(&header)), ShaderFeatures::NONE);
    }

    #[test]
    fn the_bounce_layout_needs_bounce() {
        let used = bake_features(&aux(&[(
            header::FLAGS,
            flags::BOUNCE_LAYOUT | flags::CUBE_BIAS,
        )]));
        assert!(used.bounce && !used.light_bvh);
    }

    /// The automatic choice only settles on a feature that is reported in every frame, so this
    /// also checks that the use survives `Last` clearing it.
    #[test]
    fn an_applied_bvh_bake_stays_in_the_automatic_choice() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DynamicLightingSettings {
                feature_selection: FeatureSelection::Automatic,
                ..default()
            })
            .add_plugins(SharedPlugin)
            .init_resource::<AppliedBake>()
            .add_systems(PostUpdate, note_bake_features);
        for _ in 0..3 {
            app.update();
        }
        app.world_mut().resource_mut::<AppliedBake>().buffers.aux =
            aux(&[(header::BVH_NODE_COUNT, 1)]);
        let light_bvh = ShaderFeatures {
            light_bvh: true,
            ..ShaderFeatures::NONE
        };
        for _ in 0..20 {
            app.update();
        }
        assert_eq!(
            app.world().resource::<LitShaderFeatures>().0,
            Some(light_bvh)
        );
    }
}
