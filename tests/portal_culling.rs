//! Portal culling end to end, headless: which realtime lights are packed, which get a shadow
//! cubemap and which fog volumes are drawn when rooms surround the cameras.
//!
//! Rooms 6 m wide, 3 m high and 6 m deep: A, B and C in a row along +X (doors at `x = 6`,
//! `z = 0` and at `x = 12`, `z = 2.5`), and D beside A on +Z, closed.

use bevy::camera::primitives::Frustum;
use bevy::camera::{CameraProjection, PerspectiveProjection};
use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use dynlight::fog::{FogPostProcess, FogVolumes, VolumetricShape, bounding_radius};
use dynlight::light::PackedLights;
use dynlight::portals::{RoomGraph, RoomView, sees};
use dynlight::prelude::*;
use dynlight::realtime_shadows::ShadowChoice;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, DynamicLightingPlugin));
    let world = app.world_mut();
    let size = Vec3::new(6.0, 3.0, 6.0);
    for center in [(3.0, 0.0), (9.0, 0.0), (15.0, 0.0), (3.0, 6.0)] {
        world.spawn((
            Room::new(size),
            Transform::from_xyz(center.0, 1.5, center.1),
        ));
    }
    for (x, z) in [(6.0, 0.0), (12.0, 2.5)] {
        world.spawn((
            Portal::new(Vec2::new(1.0, 2.0)),
            Transform::from_xyz(x, 1.0, z).looking_to(Vec3::NEG_X, Vec3::Y),
        ));
    }
    app
}

/// A 3D camera with fog at `eye` looking at `target`, its frustum already computed (headless
/// apps don't update frusta).
fn camera(app: &mut App, eye: Vec3, target: Vec3, order: isize) -> Entity {
    let transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
    let projection = PerspectiveProjection::default();
    let frustum = projection.compute_frustum(&GlobalTransform::from(transform));
    app.world_mut()
        .spawn((
            Camera3d::default(),
            Camera { order, ..default() },
            Projection::Perspective(projection),
            transform,
            frustum,
            FogPostProcess,
        ))
        .id()
}

fn light(app: &mut App, light: DynamicLight, at: Vec3) -> Entity {
    app.world_mut()
        .spawn((
            light,
            Transform::from_translation(at),
            InheritedVisibility::VISIBLE,
        ))
        .id()
}

fn shadowed(radius: f32) -> DynamicLight {
    DynamicLight {
        radius,
        shadows: ShadowMode::Realtime,
        ..default()
    }
}

fn packed(app: &App, light: Entity) -> bool {
    app.world()
        .resource::<PackedLights>()
        .realtime
        .contains(&light)
}

fn has_cube(app: &App, light: Entity) -> bool {
    app.world()
        .resource::<ShadowChoice>()
        .lights
        .contains(&light)
}

fn fog_volumes(app: &App) -> usize {
    app.world().resource::<FogVolumes>().0.len()
}

#[test]
fn a_light_whose_sphere_reaches_the_view_through_a_wall_is_kept_without_a_cube() {
    let cases = [
        (16, RuntimeQuality::Medium, ShaderFeatures::ALL),
        (0, RuntimeQuality::Medium, ShaderFeatures::ALL),
        (16, RuntimeQuality::IntegratedGraphics, ShaderFeatures::ALL),
        (
            16,
            RuntimeQuality::Medium,
            ShaderFeatures {
                realtime_shadows: false,
                ..ShaderFeatures::ALL
            },
        ),
    ];
    for (budget, quality, features) in cases {
        let mut app = app();
        let mut settings = app.world_mut().resource_mut::<DynamicLightingSettings>();
        settings.shadow_cubemap_budget = budget;
        settings.quality = quality;
        settings.shader_features = features;
        // In A, looking at the wall to D.
        camera(
            &mut app,
            Vec3::new(3.0, 1.5, -1.0),
            Vec3::new(3.0, 1.5, 5.0),
            0,
        );
        // In D, 2 m from the wall: its light reaches into A whatever its shadows do.
        let leaking = light(&mut app, shadowed(3.0), Vec3::new(3.0, 1.5, 5.0));
        let enclosed = light(&mut app, shadowed(1.2), Vec3::new(3.0, 1.5, 6.5));
        app.update();
        let case = format!("{budget} cubes, {quality:?}, {features:?}");
        assert!(packed(&app, leaking), "{case}");
        let cubes = budget > 0 && quality != RuntimeQuality::IntegratedGraphics;
        assert_eq!(has_cube(&app, leaking), cubes, "{case}");
        assert!(
            !packed(&app, enclosed) && !has_cube(&app, enclosed),
            "{case}"
        );

        app.world_mut().resource_mut::<PortalSettings>().enabled = false;
        app.update();
        assert!(packed(&app, enclosed), "{case}: frustum only");
        assert_eq!(has_cube(&app, enclosed), cubes, "{case}: frustum only");
    }
}

#[test]
fn a_light_only_the_second_camera_sees_keeps_its_cube_and_fog() {
    let mut app = app();
    // The viewer in A looks down the row; the offset door hides C from it.
    camera(
        &mut app,
        Vec3::new(3.0, 1.5, 0.0),
        Vec3::new(9.0, 1.5, 0.0),
        1,
    );
    let at = Vec3::new(15.0, 1.5, -2.0);
    let second = camera(&mut app, Vec3::new(17.5, 1.5, 1.0), at, 0);
    let lamp = app
        .world_mut()
        .spawn((
            shadowed(0.5),
            VolumetricLight {
                radius: 0.5,
                ..default()
            },
            Transform::from_translation(at),
            InheritedVisibility::VISIBLE,
        ))
        .id();
    app.update();
    assert!(packed(&app, lamp) && has_cube(&app, lamp));
    assert_eq!(fog_volumes(&app), 1);

    app.world_mut().get_mut::<Camera>(second).unwrap().is_active = false;
    app.update();
    assert!(!packed(&app, lamp) && !has_cube(&app, lamp));
    assert_eq!(fog_volumes(&app), 0);
}

/// Whether the cameras may see the sphere, through the rooms.
fn seen(app: &mut App, center: Vec3, radius: f32) -> bool {
    let world = app.world_mut();
    let mut cameras = world.query::<(&Frustum, Option<&RoomView>)>();
    let world = &*world;
    sees(
        world.resource::<RoomGraph>(),
        cameras.iter(world),
        center,
        radius,
    )
}

#[test]
fn cone_fog_seen_only_past_its_radius_is_kept() {
    for cutoff in [30.0, 45.0] {
        let mut app = app();
        camera(
            &mut app,
            Vec3::new(3.0, 1.5, 0.0),
            Vec3::new(9.0, 1.5, 0.0),
            0,
        );
        // In B beside the door's view, the cone opening towards it.
        let at = Vec3::new(9.0, 1.5, 2.5);
        let spot = DynamicLight {
            outer_cutoff: cutoff,
            inner_cutoff: cutoff * 0.5,
            ..DynamicLight::spot()
        };
        let fog = VolumetricLight {
            shape: VolumetricShape::ConeZ,
            radius: 1.36,
            ..default()
        };
        let bound = bounding_radius(&fog, &spot, Vec3::ONE);
        app.world_mut().spawn((
            spot,
            fog,
            Transform::from_translation(at).looking_to(Vec3::new(-1.0, 0.0, -1.0), Vec3::Y),
            InheritedVisibility::VISIBLE,
        ));
        app.update();
        // The view through the door misses the fog's radius but not the cone's rim.
        assert!(!seen(&mut app, at, fog.radius), "{cutoff}°");
        assert!(seen(&mut app, at, bound), "{cutoff}°");
        assert_eq!(fog_volumes(&app), 1, "{cutoff}°");
    }
}

/// The portal at `x`.
fn door_at(app: &mut App, x: f32) -> Entity {
    let world = app.world_mut();
    let mut portals = world.query_filtered::<(Entity, &Transform), With<Portal>>();
    let (door, _) = portals
        .iter(world)
        .find(|(_, transform)| transform.translation.x == x)
        .expect("a door there");
    door
}

#[test]
fn a_closed_door_drops_the_light_behind_it_until_it_opens() {
    let mut app = app();
    camera(
        &mut app,
        Vec3::new(3.0, 1.5, 0.0),
        Vec3::new(9.0, 1.5, 0.0),
        0,
    );
    let behind = light(&mut app, shadowed(0.5), Vec3::new(9.0, 1.5, 0.0));
    // In B too, but its sphere reaches through the wall into A's view.
    let leaking = light(&mut app, shadowed(1.0), Vec3::new(6.8, 1.5, -1.5));
    let door = door_at(&mut app, 6.0);
    app.update();
    assert!(packed(&app, behind) && has_cube(&app, behind));
    for closed in [true, false] {
        let mut entity = app.world_mut().entity_mut(door);
        if closed {
            entity.insert(ClosedPortal);
        } else {
            entity.remove::<ClosedPortal>();
        }
        app.update();
        assert_eq!(packed(&app, behind), !closed);
        assert_eq!(has_cube(&app, behind), !closed);
        assert!(packed(&app, leaking) && has_cube(&app, leaking), "{closed}");
    }
}
