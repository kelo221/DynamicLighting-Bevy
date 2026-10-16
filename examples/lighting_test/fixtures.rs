//! The lab's fog volumes and light fixtures: a sphere, a cone and a turned box of fog, a
//! fluorescent ceiling panel whose light falls through a square fog shaft onto a few props, and
//! a flickering tube light on the wall.

use core::f32::consts::FRAC_PI_2;

use bevy::prelude::*;
use dynlight::prelude::*;

use crate::box_with_lightmap_uvs;
use crate::player::Solid;

/// Where the ceiling panel hangs, and its size (long along X).
const PANEL: Vec3 = Vec3::new(-6.0, 3.2, 10.0);
const PANEL_SIZE: Vec2 = Vec2::new(1.6, 0.4);
/// Where the wall tube hangs, just in front of the wall's face at z = −5.75.
const TUBE: Vec3 = Vec3::new(16.5, 2.6, -5.6);
const TUBE_LENGTH: f32 = 1.4;

pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<DynLitMaterial>,
    prop: Handle<DynLitMaterial>,
) {
    spawn_fog(commands);
    spawn_panel(commands, meshes, materials, prop);
    spawn_tube(commands, meshes, materials);
}

/// A black surface that glows: the visible part of a lamp.
fn glow(materials: &mut Assets<DynLitMaterial>, color: LinearRgba) -> Handle<DynLitMaterial> {
    materials.add(DynLitMaterial {
        base_color: Color::BLACK,
        emissive: color,
        ..default()
    })
}

fn spawn_fog(commands: &mut Commands) {
    commands.spawn((
        Name::new("Fog"),
        DynamicLight {
            color: Color::srgb(0.4, 0.6, 1.0),
            radius: 2.0,
            realtime: true,
            ..default()
        },
        VolumetricLight {
            radius: 2.0,
            ..default()
        },
        LightEffect::new(LightEffectKind::Pulse),
        Transform::from_xyz(-12.0, 1.5, -0.5),
    ));
    commands.spawn((
        Name::new("Cone fog"),
        DynamicLight {
            light_type: LightType::Spot,
            color: Color::srgb(1.0, 0.85, 0.5),
            radius: 4.0,
            inner_cutoff: 20.0,
            outer_cutoff: 25.0,
            realtime: true,
            ..default()
        },
        VolumetricLight {
            shape: VolumetricShape::ConeZ,
            radius: 3.5,
            intensity: 0.5,
            ..default()
        },
        Transform::from_xyz(12.0, 3.5, 1.5).looking_to(Dir3::NEG_Y, Vec3::NEG_Z),
    ));
    commands.spawn((
        Name::new("Box fog"),
        DynamicLight {
            color: Color::srgb(0.6, 1.0, 0.6),
            radius: 2.0,
            realtime: true,
            ..default()
        },
        VolumetricLight {
            shape: VolumetricShape::Box,
            radius: 1.0,
            intensity: 0.5,
            ..default()
        },
        Transform::from_xyz(-12.0, 1.0, 4.5)
            .with_rotation(Quat::from_rotation_y(0.5))
            .with_scale(Vec3::new(1.5, 1.0, 1.0)),
    ));
}

/// A fluorescent panel over a crate and a pillar. The props get baked shadows; the cube you drive
/// casts a realtime one, redrawn only while it moves under the panel.
fn spawn_panel(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<DynLitMaterial>,
    prop: Handle<DynLitMaterial>,
) {
    commands.spawn((
        Name::new("Ceiling panel"),
        DynamicLight {
            color: Color::srgb(0.85, 0.93, 1.0),
            intensity: 2.5,
            radius: 6.0,
            // How fast the fog shaft widens.
            outer_cutoff: 12.0,
            shadows: ShadowMode::Realtime,
            ..DynamicLight::rectangle(PANEL_SIZE.x, PANEL_SIZE.y)
        },
        LightEffect::new(LightEffectKind::FluorescentStarter),
        VolumetricLight {
            shape: VolumetricShape::Frustum,
            radius: PANEL.y,
            intensity: 0.3,
            ..default()
        },
        Transform::from_translation(PANEL).looking_to(Dir3::NEG_Y, Vec3::NEG_Z),
    ));
    commands.spawn((
        Name::new("Ceiling panel fixture"),
        Mesh3d(meshes.add(Cuboid::new(PANEL_SIZE.x, 0.04, PANEL_SIZE.y))),
        MeshMaterial3d(glow(materials, LinearRgba::rgb(2.4, 2.7, 3.0))),
        Transform::from_translation(PANEL + Vec3::Y * 0.03),
    ));
    let props = [
        (
            PANEL.with_y(0.35) + Vec3::new(-0.9, 0.0, -0.3),
            Vec3::splat(0.7),
            0.4,
        ),
        (
            PANEL.with_y(0.7) + Vec3::new(0.8, 0.0, 0.4),
            Vec3::new(0.35, 1.4, 0.35),
            0.25,
        ),
    ];
    for (i, (center, size, solid)) in props.into_iter().enumerate() {
        commands.spawn((
            Name::new(format!("Panel prop {i}")),
            Mesh3d(meshes.add(box_with_lightmap_uvs(Cuboid::from_size(size)))),
            MeshMaterial3d(prop.clone()),
            Transform::from_translation(center).with_rotation(Quat::from_rotation_y(0.3)),
            BakeStatic,
            Solid(solid),
        ));
    }
}

/// A tube on the wall that flickers at random, lighting a streak of wall around it.
fn spawn_tube(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<DynLitMaterial>,
) {
    commands.spawn((
        Name::new("Wall tube"),
        DynamicLight {
            color: Color::srgb(0.8, 1.0, 0.88),
            intensity: 1.6,
            radius: 4.0,
            ..DynamicLight::tube(TUBE_LENGTH)
        },
        LightEffect::new(LightEffectKind::FluorescentRandom),
        Transform::from_translation(TUBE),
    ));
    commands.spawn((
        Name::new("Wall tube fixture"),
        Mesh3d(meshes.add(Capsule3d::new(0.03, TUBE_LENGTH))),
        MeshMaterial3d(glow(materials, LinearRgba::rgb(2.2, 2.8, 2.4))),
        Transform::from_translation(TUBE).with_rotation(Quat::from_rotation_z(FRAC_PI_2)),
    ));
}
