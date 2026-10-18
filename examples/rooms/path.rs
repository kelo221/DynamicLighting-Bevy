//! `--path`: a flythrough through doors and back that repeats exactly, and the lights that bob
//! unless `--still`.

use bevy::prelude::*;

use crate::grid::grid_door;
use crate::layout::{Bob, NORTH_DOORS, ROOM, SOUTH_DOORS};

/// Meters per second along the path.
const SPEED: f32 = 1.6;
const EYE: f32 = 1.6;
/// The camera looks where it will be this many seconds later.
const LOOK_AHEAD: f32 = 1.0;
/// Seconds of path per frame while `--frames` measures, so every run sees the same frames.
pub const FRAME_STEP: f32 = 1.0 / 60.0;

/// A closed loop of waypoints at eye height, and how far along it the camera is.
#[derive(Resource)]
pub struct Path {
    points: Vec<Vec3>,
    lengths: Vec<f32>,
    total: f32,
    seconds: f32,
    /// With `--frames`: real seconds before the camera starts, then [`FRAME_STEP`] per frame.
    pub fixed_after: Option<f32>,
}

impl Path {
    pub fn new(grid: Option<u32>) -> Self {
        let points: Vec<Vec2> = match grid {
            None => corridor_tour(),
            Some(n) => grid_tour(n),
        };
        let points: Vec<Vec3> = points.iter().map(|p| Vec3::new(p.x, EYE, p.y)).collect();
        let lengths: Vec<f32> = (0..points.len())
            .map(|i| points[i].distance(points[(i + 1) % points.len()]))
            .collect();
        Self {
            total: lengths.iter().sum::<f32>().max(1.0e-3),
            points,
            lengths,
            seconds: 0.0,
            fixed_after: None,
        }
    }

    fn at(&self, seconds: f32) -> Vec3 {
        let mut distance = (seconds * SPEED).rem_euclid(self.total);
        for (index, &length) in self.lengths.iter().enumerate() {
            if distance <= length && length > 0.0 {
                let next = self.points[(index + 1) % self.points.len()];
                return self.points[index].lerp(next, distance / length);
            }
            distance -= length;
        }
        self.points[0]
    }

    pub fn transform(&self) -> Transform {
        let eye = self.at(self.seconds);
        let ahead = self.at(self.seconds + LOOK_AHEAD);
        let target = if ahead.distance(eye) > 0.1 {
            ahead
        } else {
            eye + Vec3::X
        };
        Transform::from_translation(eye).looking_at(target.with_y(EYE - 0.2), Vec3::Y)
    }
}

/// Down the corridor, into a north room and a south room and out again, and back.
fn corridor_tour() -> Vec<Vec2> {
    let north = ROOM + ROOM / 2.0 + NORTH_DOORS[1];
    let south = 2.0 * ROOM + ROOM / 2.0 + SOUTH_DOORS[2];
    vec![
        Vec2::new(1.5, -0.5),
        Vec2::new(north, -0.3),
        Vec2::new(north, 3.0),
        Vec2::new(north - 2.5, 6.5),
        Vec2::new(north + 0.5, 7.0),
        Vec2::new(north, 3.0),
        Vec2::new(north, 0.0),
        Vec2::new(south, 0.3),
        Vec2::new(south, -3.0),
        Vec2::new(south + 1.0, -6.5),
        Vec2::new(south, -3.0),
        Vec2::new(south, 0.0),
        Vec2::new(22.5, 0.5),
        Vec2::new(1.5, 0.5),
    ]
}

/// Through the east doors of the first row of rooms, and back.
fn grid_tour(n: u32) -> Vec<Vec2> {
    if n < 2 {
        return vec![
            Vec2::new(2.0, 2.0),
            Vec2::new(6.0, 2.0),
            Vec2::new(6.0, 6.0),
            Vec2::new(2.0, 6.0),
        ];
    }
    let mut out = vec![Vec2::new(2.0, 2.0)];
    for i in 1..n {
        let z = ROOM / 2.0 + grid_door(i, 0, true);
        let x = ROOM * i as f32;
        out.extend([Vec2::new(x - 1.2, z), Vec2::new(x + 1.2, z)]);
    }
    out.push(Vec2::new(ROOM * n as f32 - 2.0, ROOM / 2.0));
    let back: Vec<Vec2> = out[1..out.len() - 1].iter().rev().copied().collect();
    out.extend(back);
    out
}

/// Moves the camera along the path: by virtual time, or by [`FRAME_STEP`] per frame once
/// `--frames` starts measuring.
pub fn fly(
    mut path: ResMut<Path>,
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    let step = match path.fixed_after {
        Some(after) if real.elapsed_secs() < after => 0.0,
        Some(_) => FRAME_STEP,
        None => time.delta_secs(),
    };
    path.seconds += step;
    for mut transform in &mut cameras {
        *transform = path.transform();
    }
}

pub fn bob(time: Res<Time>, mut lights: Query<(&Bob, &mut Transform)>) {
    let seconds = time.elapsed_secs();
    for (bob, mut transform) in &mut lights {
        transform.translation = bob.anchor + Vec3::Y * 0.2 * (seconds * 1.3 + bob.phase).sin();
    }
}
