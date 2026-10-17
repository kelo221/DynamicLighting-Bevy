//! Light grid benchmark: builds the realtime light clusters for 100, 1000 and 10 000 point
//! lights scattered over a 60 m square, seen by a 16:9 camera above one edge. Prints stable
//! `dynlight bench:` lines with the build time and list sizes.
//!
//! `cargo bench --bench clusters`. Times are the best of 20 builds on one thread.

use std::time::{Duration, Instant};

use bevy::camera::{CameraProjection, PerspectiveProjection};
use bevy::math::Vec3;
use bevy::prelude::Transform;
use dynlight::effects::EffectRng;
use dynlight::light::RealtimeViewer;
use dynlight::light_clusters::build;
use dynlight::shared::{ChannelFlags, GpuLight};

const RUNS: usize = 20;
const HALF_SIZE: f32 = 30.0;

fn lights(count: usize) -> Vec<GpuLight> {
    let mut rng = EffectRng::new(5, 11);
    let channel = ChannelFlags {
        realtime: true,
        ..Default::default()
    }
    .pack();
    (0..count)
        .map(|_| {
            let x = (rng.next_f32() * 2.0 - 1.0) * HALF_SIZE;
            let z = (rng.next_f32() * 2.0 - 1.0) * HALF_SIZE;
            let radius = 3.0 + 3.0 * rng.next_f32();
            GpuLight {
                position: [x, 0.5 + 2.5 * rng.next_f32(), z],
                radius_sqr: radius * radius,
                channel,
                intensity: 1.0,
                ..Default::default()
            }
        })
        .collect()
}

fn main() {
    let transform = Transform::from_xyz(0.0, 8.0, HALF_SIZE + 4.0)
        .looking_at(Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
    let viewer = RealtimeViewer {
        view_from_world: transform.to_matrix().inverse(),
        clip_from_view: PerspectiveProjection {
            aspect_ratio: 16.0 / 9.0,
            ..Default::default()
        }
        .get_clip_from_view(),
    };
    for count in [100, 1000, 10_000] {
        let lights = lights(count);
        let mut best = Duration::MAX;
        let mut entries = 0;
        for _ in 0..RUNS {
            let start = Instant::now();
            let lists = std::hint::black_box(build(Some(viewer), &lights, 0));
            best = best.min(start.elapsed());
            entries = lists.map(|lists| lists.entries).unwrap_or(0);
        }
        println!(
            "dynlight bench: clusters lights {count}, entries {entries}, cells {}, us {}",
            dynlight::light_clusters::GRID.element_product(),
            best.as_micros()
        );
    }
}
