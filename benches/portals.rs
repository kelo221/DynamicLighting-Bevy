//! Rooms-and-portals benchmark: `n × n` rooms of 8 m, each joined to its neighbours by a door,
//! with three light spheres a room (radius 5.5, 3 and 2.5 m) and a camera in a corner room
//! looking across the grid, as in `rooms --grid n`. Prints stable `dynlight bench:` lines with
//! the time to rebuild the graph, to trace the camera's view, and to test every sphere through
//! the rooms against the frustum alone.
//!
//! `cargo bench --bench portals`. Times are medians over many frames on one thread; the trace
//! is the extra time of an app update with portal culling on over one with it off.

use std::time::{Duration, Instant};

use bevy::camera::CameraProjection;
use bevy::camera::primitives::Frustum;
use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use dynlight::portals::{Portal, PortalSettings, PortalStats, PortalsPlugin, Room, RoomGraph};
use dynlight::portals::{RoomView, sees};

const ROOM: f32 = 8.0;
const FRAMES: usize = 2000;
const SPHERE_RUNS: usize = 200;

/// A stable offset in `-2.5..2.5` m for door `index`.
fn offset(index: u32) -> f32 {
    let mut x = index.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    (x & 0xffff) as f32 / 65_535.0 * 5.0 - 2.5
}

/// The rooms and doors of an `n × n` grid, and the light spheres in it.
fn grid(world: &mut World, n: u32) -> Vec<(Vec3, f32)> {
    let mut spheres = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let center = Vec3::new(ROOM * (i as f32 + 0.5), 1.5, ROOM * (j as f32 + 0.5));
            world.spawn((
                Room::new(Vec3::new(ROOM, 3.0, ROOM)),
                Transform::from_translation(center),
            ));
            spheres.push((center + Vec3::new(0.0, 0.9, 0.3), 5.5));
            spheres.push((center + Vec3::new(1.8, 0.4, 2.2), 3.0));
            spheres.push((center + Vec3::new(-3.0, 0.2, -1.0), 2.5));
            let middle = |k: u32| ROOM * (k as f32 + 0.5);
            if i > 0 {
                let along = middle(j) + offset(i * 4099 + j * 8 + 1);
                world.spawn((
                    Portal::new(Vec2::new(1.2, 2.2)),
                    Transform::from_xyz(ROOM * i as f32, 1.1, along).looking_to(Vec3::X, Vec3::Y),
                ));
            }
            if j > 0 {
                let along = middle(i) + offset(i * 4099 + j * 8);
                world.spawn((
                    Portal::new(Vec2::new(1.2, 2.2)),
                    Transform::from_xyz(along, 1.1, ROOM * j as f32),
                ));
            }
        }
    }
    spheres
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

/// The median time of an app update with portal culling `enabled`.
fn update_time(app: &mut App, enabled: bool) -> Duration {
    app.world_mut().resource_mut::<PortalSettings>().enabled = enabled;
    app.update();
    let times = (0..FRAMES)
        .map(|_| {
            let start = Instant::now();
            app.update();
            start.elapsed()
        })
        .collect();
    median(times)
}

/// The median time to rebuild the graph, from updates that change the tolerance a little.
fn rebuild_time(app: &mut App, still: Duration) -> Duration {
    let times = (0..FRAMES / 10)
        .map(|frame| {
            app.world_mut().resource_mut::<PortalSettings>().tolerance =
                0.02 + 1.0e-4 * (frame % 2) as f32;
            let start = Instant::now();
            app.update();
            start.elapsed()
        })
        .collect();
    median(times).saturating_sub(still)
}

fn main() {
    for n in [6, 16] {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, PortalsPlugin));
        let spheres = grid(app.world_mut(), n);
        let far = ROOM * n as f32 - 1.0;
        let transform =
            Transform::from_xyz(1.0, 1.7, 1.0).looking_at(Vec3::new(far, 1.2, far), Vec3::Y);
        let projection = PerspectiveProjection {
            aspect_ratio: 16.0 / 9.0,
            ..default()
        };
        let frustum = projection.compute_frustum(&GlobalTransform::from(transform));
        app.world_mut().spawn((
            Camera::default(),
            Projection::Perspective(projection),
            transform,
            frustum,
        ));

        let off = update_time(&mut app, false);
        let on = update_time(&mut app, true);
        let rebuild = rebuild_time(&mut app, on);
        let world = app.world_mut();
        let stats = world.resource::<PortalStats>().clone();
        let mut cameras = world.query::<(&Frustum, &RoomView)>();
        let world = &*world;
        let (frustum, view) = cameras.single(world).expect("one camera");
        let rooms = world.resource::<RoomGraph>();
        let test = |view: Option<&RoomView>| {
            let mut kept = 0;
            let times = (0..SPHERE_RUNS)
                .map(|_| {
                    let start = Instant::now();
                    kept = spheres
                        .iter()
                        .filter(|&&(center, radius)| sees(rooms, [(frustum, view)], center, radius))
                        .count();
                    start.elapsed()
                })
                .collect();
            (kept, median(times))
        };
        let (kept, through) = test(Some(view));
        let (framed, alone) = test(None);
        println!(
            "dynlight bench: portals rooms {}, portals {}, visits {}, portal tests {}, plane tests \
             {}, rebuild us {}, trace us {:.1}, spheres {}, kept {kept} (frustum {framed}), sees us \
             {:.1} (frustum only {:.1})",
            stats.rooms,
            stats.portals,
            stats.visits,
            stats.portal_tests,
            stats.plane_tests,
            rebuild.as_micros(),
            on.saturating_sub(off).as_secs_f64() * 1.0e6,
            spheres.len(),
            through.as_secs_f64() * 1.0e6,
            alone.as_secs_f64() * 1.0e6,
        );
    }
}
