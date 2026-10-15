//! The cube you drive around the lab: WASD or the arrow keys move it relative to the camera
//! (key positions, so Colemak's WARS works too), Shift runs, and Space switches the lantern it
//! carries, a realtime light whose shadow cubemap makes the props cast moving shadows.

use bevy::prelude::*;
use dynlight::prelude::*;

/// Meters per second; Shift doubles it.
const SPEED: f32 = 3.0;
/// How quickly the cube reaches the speed the keys ask for, per second.
const ACCELERATION: f32 = 10.0;
/// How quickly the cube turns to face where it goes, per second.
const TURN_RATE: f32 = 8.0;
const SIZE: f32 = 0.5;
/// Footprint radius for bumping into props; a little under the half-diagonal.
const RADIUS: f32 = 0.32;
/// Where the cube may go: the floor, in front of the wall.
const BOUNDS: (Vec2, Vec2) = (Vec2::new(-19.5, -5.4), Vec2::new(19.5, 19.5));

#[derive(Component, Default)]
pub struct Player {
    velocity: Vec3,
}

#[derive(Component)]
struct Lantern;

/// A prop the cube bumps into, as a circle of this radius on the floor.
#[derive(Component)]
pub struct Solid(pub f32);

pub fn plugin(app: &mut App) {
    app.add_systems(Update, (drive, switch_lantern));
}

/// Spawns the cube and its lantern at `position` on the floor.
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<DynLitMaterial>,
    position: Vec2,
) -> Entity {
    commands
        .spawn((
            Name::new("Player"),
            Player::default(),
            Mesh3d(meshes.add(Cuboid::from_length(SIZE))),
            MeshMaterial3d(materials.add(Color::srgb(0.25, 0.75, 0.85))),
            Transform::from_xyz(position.x, SIZE / 2.0, position.y),
            children![(
                Name::new("Lantern"),
                Lantern,
                DynamicLight {
                    color: Color::srgb(1.0, 0.82, 0.6),
                    radius: 5.0,
                    intensity: 1.6,
                    realtime: true,
                    shadows: ShadowMode::Realtime,
                    ..DynamicLight::point()
                },
                Transform::from_xyz(0.0, 0.9, 0.0),
            )],
        ))
        .id()
}

/// The keys' direction: x to the right, y forward.
fn wish(keys: &ButtonInput<KeyCode>) -> Vec2 {
    let axis = |positive: [KeyCode; 2], negative: [KeyCode; 2]| {
        f32::from(keys.any_pressed(positive)) - f32::from(keys.any_pressed(negative))
    };
    Vec2::new(
        axis(
            [KeyCode::KeyD, KeyCode::ArrowRight],
            [KeyCode::KeyA, KeyCode::ArrowLeft],
        ),
        axis(
            [KeyCode::KeyW, KeyCode::ArrowUp],
            [KeyCode::KeyS, KeyCode::ArrowDown],
        ),
    )
    .normalize_or_zero()
}

fn drive(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    cameras: Query<&Transform, (With<Camera3d>, Without<Player>)>,
    mut players: Query<(&mut Player, &mut Transform), Without<Solid>>,
    solids: Query<(&Transform, &Solid)>,
) {
    let seconds = time.delta_secs();
    let input = wish(&keys);
    let forward = cameras
        .iter()
        .next()
        .map_or(Vec3::NEG_Z, |camera| *camera.forward())
        .with_y(0.0)
        .normalize_or(Vec3::NEG_Z);
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let run = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        2.0
    } else {
        1.0
    };
    let wanted = (right * input.x + forward * input.y) * SPEED * run;
    for (mut player, mut transform) in &mut players {
        player.velocity = player
            .velocity
            .lerp(wanted, 1.0 - (-ACCELERATION * seconds).exp());
        let mut position = transform.translation.xz() + player.velocity.xz() * seconds;
        for (solid, Solid(radius)) in &solids {
            let center = solid.translation.xz();
            let apart = position - center;
            let reach = RADIUS + radius;
            if apart.length_squared() < reach * reach {
                position = center + apart.normalize_or(Vec2::X) * reach;
            }
        }
        let position = position.clamp(BOUNDS.0, BOUNDS.1);
        transform.translation = Vec3::new(position.x, SIZE / 2.0, position.y);
        if player.velocity.length_squared() > 0.01 {
            let facing = Quat::from_rotation_y(player.velocity.x.atan2(player.velocity.z));
            let turn = 1.0 - (-TURN_RATE * seconds).exp();
            transform.rotation = transform.rotation.slerp(facing, turn);
        }
    }
}

fn switch_lantern(
    keys: Res<ButtonInput<KeyCode>>,
    mut lanterns: Query<&mut Visibility, With<Lantern>>,
) {
    if !keys.just_pressed(KeyCode::Space) {
        return;
    }
    for mut visibility in &mut lanterns {
        visibility.toggle_visible_hidden();
    }
}
