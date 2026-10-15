//! Light cookies: grayscale textures projected by spot lights, deduplicated into one texture
//! array that every lit material binds.
//!
//! Add [`LightCookie`] next to a spot [`DynamicLight`]. The image must stay readable on the CPU
//! (the default `RenderAssetUsages`); it is resampled to 1024² and its linear red channel used.

mod layers;

use bevy::asset::{AssetEvent, uuid_handle};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;

pub use layers::{COOKIE_BUDGET, COOKIE_RESOLUTION, array_image, resample};

use crate::light::{DynamicLight, LightAttachments, LightSystems, LightType};

/// The cookie texture array bound by `DynLitMaterial`.
pub const COOKIE_ARRAY: Handle<Image> = uuid_handle!("8f0a2d61-3c7e-4b59-a1d4-6e93b2c5f07a");

/// A texture projected by a spot light. Ignored on other light types, and on spot lights whose
/// `outer_cutoff` is 0° or above 90°, where the projection is undefined.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Debug, Clone, PartialEq)]
pub struct LightCookie(pub Handle<Image>);

/// Which image is in which array layer.
#[derive(Resource, Default, Debug)]
pub struct CookieLayers {
    pub images: Vec<AssetId<Image>>,
}

pub struct CookiesPlugin;

impl Plugin for CookiesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<LightCookie>()
            .init_resource::<CookieLayers>();
    }

    fn finish(&self, app: &mut App) {
        let world = app.world_mut();
        let Some(mut images) = world.get_resource_mut::<Assets<Image>>() else {
            return;
        };
        images
            .insert(&COOKIE_ARRAY, array_image(&[], COOKIE_RESOLUTION))
            .expect("the cookie array handle is a fixed UUID");
        app.add_systems(PostUpdate, assign_cookies.in_set(LightSystems::Attach));
    }
}

fn assign_cookies(
    lights: Query<(Entity, &LightCookie, &DynamicLight)>,
    mut events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
    mut layers: ResMut<CookieLayers>,
    mut attachments: ResMut<LightAttachments>,
) {
    let mut dirty = events.read().any(|event| match event {
        AssetEvent::Modified { id } => *id != COOKIE_ARRAY.id() && layers.images.contains(id),
        _ => false,
    });
    attachments.cookies.clear();
    let projecting = || {
        lights
            .iter()
            .filter(|(_, cookie, light)| projects_cookie(light) && images.contains(cookie.0.id()))
    };
    let in_use: HashSet<AssetId<Image>> =
        projecting().map(|(_, cookie, _)| cookie.0.id()).collect();
    for (entity, cookie, _) in projecting() {
        let Some((index, added)) =
            cookie_layer(&mut layers.images, cookie.0.id(), &in_use, COOKIE_BUDGET)
        else {
            continue;
        };
        dirty |= added;
        attachments.cookies.insert(entity, index as u32);
    }
    if !dirty {
        return;
    }
    let resampled: Vec<Vec<u8>> = layers
        .images
        .iter()
        .map(|id| {
            images
                .get(*id)
                .and_then(|image| resample(image, COOKIE_RESOLUTION))
                .unwrap_or_else(|| {
                    warn!("dynlight: cookie image {id:?} isn't readable on the CPU; using white");
                    vec![255; (COOKIE_RESOLUTION * COOKIE_RESOLUTION) as usize]
                })
        })
        .collect();
    let _ = images.insert(&COOKIE_ARRAY, array_image(&resampled, COOKIE_RESOLUTION));
}

/// Whether `light` can project a cookie: a valid spot light with a cone between 0° and 90°
/// (Henry's `LightCookieProcessLight` guard).
fn projects_cookie(light: &DynamicLight) -> bool {
    light.light_type == LightType::Spot
        && light.is_valid()
        && light.outer_cutoff > 0.0
        && light.outer_cutoff <= 90.0
}

/// The array layer of image `id`, and whether it was newly added. New images take a free
/// layer, or the layer of an image no light uses any more (`in_use`); `None` when every one of
/// the `budget` layers is in use.
fn cookie_layer<K: Copy + Eq + core::hash::Hash>(
    layers: &mut Vec<K>,
    id: K,
    in_use: &HashSet<K>,
    budget: usize,
) -> Option<(usize, bool)> {
    if let Some(index) = layers.iter().position(|image| *image == id) {
        return Some((index, false));
    }
    if layers.len() < budget {
        layers.push(id);
        return Some((layers.len() - 1, true));
    }
    let unused = layers.iter().position(|image| !in_use.contains(image))?;
    layers[unused] = id;
    Some((unused, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unused_layers_are_reused_once_the_budget_is_full() {
        let mut layers = Vec::new();
        let in_use: HashSet<u32> = [1, 2, 3].into();
        assert_eq!(cookie_layer(&mut layers, 1, &in_use, 2), Some((0, true)));
        assert_eq!(cookie_layer(&mut layers, 2, &in_use, 2), Some((1, true)));
        assert_eq!(cookie_layer(&mut layers, 1, &in_use, 2), Some((0, false)));
        assert_eq!(cookie_layer(&mut layers, 3, &in_use, 2), None, "all in use");
        let in_use: HashSet<u32> = [2, 3].into();
        assert_eq!(cookie_layer(&mut layers, 3, &in_use, 2), Some((0, true)));
        assert_eq!(layers, [3, 2]);
    }

    #[test]
    fn only_spot_cones_up_to_90_degrees_project() {
        let spot = |outer_cutoff: f32| DynamicLight {
            inner_cutoff: outer_cutoff.min(10.0),
            outer_cutoff,
            ..DynamicLight::spot()
        };
        assert!(projects_cookie(&spot(30.0)));
        assert!(projects_cookie(&spot(90.0)));
        assert!(!projects_cookie(&spot(95.0)));
        assert!(!projects_cookie(&spot(0.0)));
        assert!(!projects_cookie(&DynamicLight::point()));
    }
}
