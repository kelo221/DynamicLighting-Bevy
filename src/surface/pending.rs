//! Baked meshes whose [`DynLitMaterial`] or base color texture is still loading get a
//! [`SurfacePending`]: until they arrive, whether the mesh is blended, its alpha mask and the
//! color it reflects aren't known, so bakes wait and stale-shadow checks hold off.

use bevy::asset::UntypedAssetId;
use bevy::prelude::*;

use crate::bake::{BakeStatic, SurfacePending};

use super::material::DynLitMaterial;

type Surface<'a> = (
    Entity,
    &'a MeshMaterial3d<DynLitMaterial>,
    Has<SurfacePending>,
);

type Sources<'w> = (
    Res<'w, Assets<DynLitMaterial>>,
    Res<'w, Assets<Image>>,
    Option<Res<'w, AssetServer>>,
);

/// Whether `material` (or its base color texture) is still to arrive. Failed loads count as
/// arrived, so nothing waits for them forever.
fn waiting(
    material: &Handle<DynLitMaterial>,
    materials: &Assets<DynLitMaterial>,
    images: &Assets<Image>,
    server: Option<&AssetServer>,
) -> bool {
    let failed = |id: UntypedAssetId| server.is_some_and(|s| s.load_state(id).is_failed());
    let Some(lit) = materials.get(material) else {
        return !failed(material.id().untyped());
    };
    lit.base_color_texture
        .as_ref()
        .is_some_and(|texture| images.get(texture).is_none() && !failed(texture.id().untyped()))
}

/// Keeps [`SurfacePending`] on the [`BakeStatic`] meshes whose surface is still loading.
pub(super) fn sync_surface_pending(
    mut commands: Commands,
    (mut material_events, mut image_events): (
        MessageReader<AssetEvent<DynLitMaterial>>,
        MessageReader<AssetEvent<Image>>,
    ),
    added: Query<(), (With<BakeStatic>, Added<MeshMaterial3d<DynLitMaterial>>)>,
    pending: Query<(), With<SurfacePending>>,
    (materials, images, server): Sources,
    meshes: Query<Surface, With<BakeStatic>>,
) {
    let assets_changed = material_events.read().count() > 0 || image_events.read().count() > 0;
    if !assets_changed && added.is_empty() && pending.is_empty() {
        return;
    }
    for (entity, material, marked) in &meshes {
        let wanted = waiting(&material.0, &materials, &images, server.as_deref());
        if wanted && !marked {
            commands.entity(entity).insert(SurfacePending);
        } else if !wanted && marked {
            commands.entity(entity).remove::<SurfacePending>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meshes_wait_for_their_texture() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Image>()
            .init_asset::<DynLitMaterial>()
            .add_systems(Update, sync_surface_pending);
        let world = app.world_mut();
        let texture = world.resource_mut::<Assets<Image>>().reserve_handle();
        let material = DynLitMaterial {
            base_color_texture: Some(texture.clone()),
            ..default()
        };
        let material = world.resource_mut::<Assets<DynLitMaterial>>().add(material);
        let mesh = world.spawn((BakeStatic, MeshMaterial3d(material))).id();
        app.update();
        let pending = |app: &App| app.world().entity(mesh).contains::<SurfacePending>();
        assert!(pending(&app), "the texture isn't there yet");
        let image = Image::default();
        let world = app.world_mut();
        let _ = world
            .resource_mut::<Assets<Image>>()
            .insert(&texture, image);
        app.update();
        assert!(!pending(&app), "the texture arrived");
    }
}
