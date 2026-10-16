//! Shadow casters whose material changed: they switched to another material, theirs was edited
//! (alpha mode, cutoff, color), or one of its textures (re)loaded. A material type opts in with
//! [`CasterMaterialPlugin`]; the surface slice adds it for `DynLitMaterial`.

use core::marker::PhantomData;

use bevy::asset::AssetEventSystems;
use bevy::asset::asset_changed::AssetChanged;
use bevy::pbr::{Material, MeshMaterial3d};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;

use super::tracking::{TouchedCasters, track_casters};

/// A material whose shadow casters the shadow cubemaps follow.
pub trait CasterMaterial: Material {
    /// Calls `visit` with each texture of the material itself, leaving out scene-wide resources
    /// such as the light buffers. A caster isn't drawn until its material's textures are on the
    /// GPU, and an alpha-masked caster cuts its silhouette from them.
    fn visit_textures(&self, visit: &mut dyn FnMut(AssetId<Image>));
}

/// Renders the shadow cubemap faces that see a caster with material `M` again when the caster
/// switches material, the material changes, or one of its textures (re)loads.
pub struct CasterMaterialPlugin<M>(PhantomData<M>);

impl<M> Default for CasterMaterialPlugin<M> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<M: CasterMaterial> Plugin for CasterMaterialPlugin<M> {
    fn build(&self, _: &mut App) {}

    fn finish(&self, app: &mut App) {
        let world = app.world();
        if !world.contains_resource::<Assets<M>>() || !world.contains_resource::<Assets<Image>>() {
            return;
        }
        app.add_systems(
            PostUpdate,
            touch_casters::<M>
                .after(AssetEventSystems)
                .before(track_casters),
        );
    }
}

/// The image an event says was added, changed, (re)loaded or removed.
fn changed_image(event: &AssetEvent<Image>) -> Option<AssetId<Image>> {
    match *event {
        AssetEvent::Added { id }
        | AssetEvent::Modified { id }
        | AssetEvent::LoadedWithDependencies { id }
        | AssetEvent::Removed { id } => Some(id),
        _ => None,
    }
}

type Switched<M> = (
    With<Mesh3d>,
    Or<(Changed<MeshMaterial3d<M>>, AssetChanged<MeshMaterial3d<M>>)>,
);

/// Reports casters with material `M` whose material is new, edited, or has new textures.
/// Without realtime shadows (headless apps) there is nothing to report to.
pub(super) fn touch_casters<M: CasterMaterial>(
    switched: Query<Entity, Switched<M>>,
    casters: Query<(Entity, &MeshMaterial3d<M>), With<Mesh3d>>,
    materials: Res<Assets<M>>,
    mut images: MessageReader<AssetEvent<Image>>,
    touched: Option<ResMut<TouchedCasters>>,
) {
    let Some(mut touched) = touched else {
        return;
    };
    touched.0.extend(&switched);
    let changed: HashSet<AssetId<Image>> = images.read().filter_map(changed_image).collect();
    if changed.is_empty() {
        return;
    }
    let mut textured = HashSet::new();
    for (id, material) in materials.iter() {
        material.visit_textures(&mut |texture| {
            if changed.contains(&texture) {
                textured.insert(id);
            }
        });
    }
    touched.0.extend(
        casters
            .iter()
            .filter(|(_, material)| textured.contains(&material.id()))
            .map(|(entity, _)| entity),
    );
}
