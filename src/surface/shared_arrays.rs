//! Keeps lit materials bound to the current shadow cubemap and cookie arrays and sky cube.
//!
//! Bevy prepares a material's bind group once and doesn't track the images in it. The images
//! are replaced under their fixed handles when they grow or change, so every [`DynLitMaterial`]
//! is marked modified to be prepared again; otherwise materials keep sampling the old texture
//! (the 1×1 placeholder when they were prepared before the first realtime shadow).

use bevy::asset::AssetEvent;
use bevy::prelude::*;

use super::material::DynLitMaterial;
use crate::cookies::COOKIE_ARRAY;
use crate::environment::SKY_SPECULAR;
use crate::realtime_shadows::SHADOW_CUBEMAPS;

/// Whether `event` replaced one of the arrays every lit material binds.
fn replaces_shared_array(event: &AssetEvent<Image>) -> bool {
    match event {
        AssetEvent::Added { id } | AssetEvent::Modified { id } => {
            *id == SHADOW_CUBEMAPS.id() || *id == COOKIE_ARRAY.id() || *id == SKY_SPECULAR.id()
        }
        _ => false,
    }
}

pub(super) fn rebind_shared_arrays(
    mut events: MessageReader<AssetEvent<Image>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    // Read every event so none is left for the next frame.
    let replaced = events.read().fold(false, |replaced, event| {
        replaced | replaces_shared_array(event)
    });
    if !replaced {
        return;
    }
    // Visiting a material through `iter_mut` marks it modified.
    materials.iter_mut().for_each(drop);
}

#[cfg(test)]
mod tests {
    use bevy::asset::AssetPlugin;

    use super::*;

    #[test]
    fn only_the_shared_arrays_count() {
        let other = AssetId::<Image>::Uuid {
            uuid: bevy::asset::uuid::Uuid::from_u128(7),
        };
        assert!(replaces_shared_array(&AssetEvent::Modified {
            id: SHADOW_CUBEMAPS.id()
        }));
        assert!(replaces_shared_array(&AssetEvent::Added {
            id: COOKIE_ARRAY.id()
        }));
        assert!(replaces_shared_array(&AssetEvent::Modified {
            id: SKY_SPECULAR.id()
        }));
        assert!(!replaces_shared_array(&AssetEvent::Removed {
            id: SHADOW_CUBEMAPS.id()
        }));
        assert!(!replaces_shared_array(&AssetEvent::Modified { id: other }));
    }

    #[test]
    fn replacing_an_array_marks_materials_modified() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Image>()
            .init_asset::<DynLitMaterial>()
            .add_systems(Update, rebind_shared_arrays);
        let material = app
            .world_mut()
            .resource_mut::<Assets<DynLitMaterial>>()
            .add(DynLitMaterial::default());
        app.update();
        let modified = |app: &mut App| {
            let events = app
                .world_mut()
                .resource_mut::<Messages<AssetEvent<DynLitMaterial>>>()
                .drain()
                .collect::<Vec<_>>();
            events.contains(&AssetEvent::Modified { id: material.id() })
        };
        modified(&mut app);
        app.update();
        assert!(!modified(&mut app), "nothing replaced yet");

        let _ = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .insert(&SHADOW_CUBEMAPS, Image::default());
        app.update();
        app.update();
        assert!(modified(&mut app));
    }
}
