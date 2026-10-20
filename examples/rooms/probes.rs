//! `--probes`: a reflection probe boxed to every room, and a chrome ball and a brushed metal
//! block in each, which reflect their room through its probe. Meshes here have no bake data, so
//! the probes also light them in place of the ambient color.

use bevy::prelude::*;
use dynlight::prelude::*;

use crate::layout::WALL;

pub fn plugin(app: &mut App) {
    if crate::support::flag("--probes") {
        app.add_systems(PostStartup, spawn);
    }
}

fn spawn(
    mut commands: Commands,
    rooms: Query<(&Room, &Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    let ball = meshes.add(Sphere::new(0.45).mesh().uv(48, 24));
    let block = meshes.add(Cuboid::new(0.7, 0.7, 0.7));
    let metal = |color: Color, roughness: f32| DynLitMaterial {
        base_color: color,
        shading: ShadingModel::Metallic,
        metallic: 1.0,
        perceptual_roughness: roughness,
        ..default()
    };
    let chrome = materials.add(metal(Color::srgb(0.95, 0.95, 0.95), 0.05));
    let brushed = materials.add(metal(Color::srgb(0.95, 0.78, 0.5), 0.3));
    for (room, transform) in &rooms {
        // The room cells run to the middle of the walls; the box stops at their inner faces.
        let half = room.half_size - Vec3::new(WALL / 2.0, 0.0, WALL / 2.0);
        commands.spawn((
            ReflectionProbe::default(),
            Transform::from_translation(transform.translation).with_scale(half),
        ));
        let floor = transform.translation.with_y(0.0);
        commands.spawn((
            Mesh3d(ball.clone()),
            MeshMaterial3d(chrome.clone()),
            Transform::from_translation(floor + Vec3::new(2.6, 0.45, 0.0)),
        ));
        commands.spawn((
            Mesh3d(block.clone()),
            MeshMaterial3d(brushed.clone()),
            Transform::from_translation(floor + Vec3::new(-2.6, 0.35, 0.0))
                .with_rotation(Quat::from_rotation_y(0.5)),
        ));
    }
}
