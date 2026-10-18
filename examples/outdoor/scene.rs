//! The courtyard: ground, two buildings, a pergola, trees, a wall, a lamp post and two metal
//! balls reflecting the sky, all baked; a rolling ball and a spinning crate that aren't, so they
//! take the sun's shadows from its sun map.

use core::f32::consts::{PI, TAU};

use bevy::prelude::*;
use dynlight::prelude::*;

use crate::lightmap_uvs::{box_with_lightmap_uvs, with_uv0_as_uv1};
use crate::sky;

/// How the moving objects move, so captures can hold them still.
#[derive(Component)]
pub enum Motion {
    Roll,
    Spin,
}

/// The ball rolls around this circle, starting under the pergola's slats.
const ROLL_CENTER: Vec3 = Vec3::new(1.5, 0.5, -2.0);
const ROLL_RADIUS: f32 = 3.0;

fn rolled(angle: f32) -> Transform {
    let around = Vec3::new(angle.cos(), 0.0, angle.sin()) * ROLL_RADIUS;
    Transform::from_translation(ROLL_CENTER + around)
        .with_rotation(Quat::from_rotation_y(-angle) * Quat::from_rotation_z(angle * 6.0))
}

/// A static, baked box.
fn block(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &Handle<DynLitMaterial>,
    name: &str,
    center: Vec3,
    size: Vec3,
) {
    commands.spawn((
        Name::new(name.to_owned()),
        Mesh3d(meshes.add(box_with_lightmap_uvs(Cuboid::from_size(size)))),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(center),
        BakeStatic,
    ));
}

pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<DynLitMaterial>,
    images: &mut Assets<Image>,
) {
    let ground = materials.add(Color::srgb(0.72, 0.66, 0.55));
    let plaster = materials.add(Color::srgb(0.9, 0.86, 0.78));
    let brick = materials.add(Color::srgb(0.68, 0.38, 0.3));
    let wood = materials.add(Color::srgb(0.55, 0.38, 0.24));
    let leaves = materials.add(Color::srgb(0.32, 0.5, 0.25));
    let metal = materials.add(Color::srgb(0.2, 0.2, 0.22));

    let floor = Plane3d::default().mesh().size(40.0, 40.0).build();
    commands.spawn((
        Name::new("Ground"),
        Mesh3d(meshes.add(with_uv0_as_uv1(floor))),
        MeshMaterial3d(ground),
        BakeStatic,
    ));

    let buildings = [
        (
            "House",
            Vec3::new(-7.0, 2.5, -6.0),
            Vec3::new(7.0, 5.0, 6.0),
        ),
        ("Shed", Vec3::new(8.0, 1.75, -7.5), Vec3::new(4.0, 3.5, 5.0)),
    ];
    for (name, center, size) in buildings {
        block(commands, meshes, &plaster, name, center, size);
    }
    block(
        commands,
        meshes,
        &brick,
        "Wall",
        Vec3::new(4.0, 0.6, 3.0),
        Vec3::new(7.0, 1.2, 0.3),
    );

    // The pergola: four posts and slats whose shadows stripe the ground.
    let pergola = Vec3::new(1.5, 0.0, -2.0);
    for (i, (x, z)) in [(-2.0, -1.5), (2.0, -1.5), (-2.0, 1.5), (2.0, 1.5)]
        .into_iter()
        .enumerate()
    {
        let post = pergola + Vec3::new(x, 1.3, z);
        block(
            commands,
            meshes,
            &wood,
            &format!("Post {i}"),
            post,
            Vec3::new(0.15, 2.6, 0.15),
        );
    }
    for i in 0..9 {
        let x = -2.0 + i as f32 * 0.5;
        let slat = pergola + Vec3::new(x, 2.65, 0.0);
        block(
            commands,
            meshes,
            &wood,
            &format!("Slat {i}"),
            slat,
            Vec3::new(0.12, 0.1, 3.4),
        );
    }

    let trunk = meshes.add(with_uv0_as_uv1(Cylinder::new(0.18, 2.4).mesh().build()));
    let crown = meshes.add(with_uv0_as_uv1(Sphere::new(1.3).mesh().uv(24, 16)));
    for (i, at) in [
        Vec2::new(-9.0, 5.0),
        Vec2::new(-3.0, 8.0),
        Vec2::new(10.0, 4.0),
    ]
    .into_iter()
    .enumerate()
    {
        commands.spawn((
            Name::new(format!("Trunk {i}")),
            Mesh3d(trunk.clone()),
            MeshMaterial3d(wood.clone()),
            Transform::from_xyz(at.x, 1.2, at.y),
            BakeStatic,
        ));
        commands.spawn((
            Name::new(format!("Crown {i}")),
            Mesh3d(crown.clone()),
            MeshMaterial3d(leaves.clone()),
            Transform::from_xyz(at.x, 3.2, at.y),
            BakeStatic,
        ));
    }

    // A lamp post with a warm baked lamp hanging off an arm, clear of the post's own shadow.
    let lamp = Vec3::new(-2.5, 0.0, 1.0);
    let parts = [
        ("Lamp post", Vec3::Y * 1.5, Vec3::new(0.12, 3.0, 0.12)),
        (
            "Lamp arm",
            Vec3::new(0.3, 2.94, 0.0),
            Vec3::new(0.6, 0.08, 0.08),
        ),
    ];
    for (name, offset, size) in parts {
        block(commands, meshes, &metal, name, lamp + offset, size);
    }
    commands.spawn((
        Name::new("Lamp"),
        DynamicLight {
            color: Color::srgb(1.0, 0.75, 0.45),
            intensity: 1.5,
            radius: 6.0,
            ..DynamicLight::point()
        },
        Transform::from_translation(lamp + Vec3::new(0.55, 2.8, 0.0)),
    ));

    let (sun, transform) = sun_light(Dir3::new(Vec3::new(-0.45, -0.75, -0.5)).unwrap());
    let sun = DynamicLight {
        color: Color::srgb(1.0, 0.93, 0.8),
        intensity: 2.5,
        angular_diameter: 1.0,
        ..sun
    };
    sky::spawn(commands, images, &sun, *transform.back());
    commands.spawn((Name::new("Sun"), sun, transform));

    // A polished and a brushed metal ball, which reflect the sky.
    let ball_mesh = meshes.add(with_uv0_as_uv1(Sphere::new(0.6).mesh().uv(32, 18)));
    for (name, at, color, roughness) in [
        (
            "Chrome ball",
            Vec3::new(-0.6, 0.6, 2.6),
            Color::srgb(0.95, 0.95, 0.95),
            0.05,
        ),
        (
            "Brass ball",
            Vec3::new(0.9, 0.6, 2.3),
            Color::srgb(0.95, 0.75, 0.4),
            0.35,
        ),
    ] {
        let material = materials.add(DynLitMaterial {
            shading: ShadingModel::Metallic,
            metallic: 1.0,
            perceptual_roughness: roughness,
            ..DynLitMaterial::from(color)
        });
        commands.spawn((
            Name::new(name),
            Mesh3d(ball_mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(at),
            BakeStatic,
        ));
    }

    let ball = materials.add(Color::srgb(0.85, 0.2, 0.15));
    commands.spawn((
        Name::new("Ball"),
        Mesh3d(meshes.add(Sphere::new(0.5).mesh().uv(32, 18))),
        MeshMaterial3d(ball),
        Motion::Roll,
        rolled(PI),
    ));
    let crate_material = materials.add(Color::srgb(0.8, 0.6, 0.35));
    commands.spawn((
        Name::new("Crate"),
        Mesh3d(meshes.add(Cuboid::new(0.9, 0.9, 0.9))),
        MeshMaterial3d(crate_material),
        Motion::Spin,
        Transform::from_xyz(8.5, 0.45, 3.3),
    ));
}

/// Rolls the ball through the pergola's stripes and spins the crate under a tree.
pub fn animate(time: Res<Time>, mut moving: Query<(&Motion, &mut Transform)>) {
    let seconds = time.elapsed_secs();
    for (motion, mut transform) in &mut moving {
        match motion {
            Motion::Roll => *transform = rolled(PI + seconds * TAU / 16.0),
            Motion::Spin => transform.rotation = Quat::from_rotation_y(seconds * 0.6),
        }
    }
}
