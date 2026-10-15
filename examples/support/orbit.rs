//! A mouse orbit camera for the demos: drag to orbit, scroll to zoom. After a few seconds
//! without input it drifts on its own: round in a circle, or back and forth across a yaw range.

// Each example uses a different subset.
#![allow(dead_code)]

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::transform::TransformSystems;

/// Seconds without input before the camera drifts again.
const IDLE_SECONDS: f32 = 4.0;
/// Radians per pixel dragged.
const DRAG_SPEED: f32 = 0.005;
/// Distance factor per scroll line.
const ZOOM_STEP: f32 = 0.9;
/// Pixels per scroll line, for browsers and touchpads that scroll in pixels.
const PIXELS_PER_LINE: f32 = 100.0;
/// How quickly a followed target pulls the orbit center along, per second.
const FOLLOW_RATE: f32 = 4.0;

/// Orbits `target`; yaw 0 looks from +Z towards −Z, positive pitch looks down.
#[derive(Component, Clone, Debug)]
pub struct OrbitCamera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    /// The idle drift sweeps back and forth across this range, and dragging stops at its ends.
    /// `None` circles freely.
    pub yaw_range: Option<(f32, f32)>,
    pub pitch_range: (f32, f32),
    pub distance_range: (f32, f32),
    /// Fastest idle drift, in radians per second.
    pub drift: f32,
    idle: f32,
    /// Where the back-and-forth sweep is, as the angle of a sine; `None` after input.
    phase: Option<f32>,
}

impl OrbitCamera {
    pub fn new(target: Vec3, yaw: f32, pitch: f32, distance: f32) -> Self {
        Self {
            target,
            yaw,
            pitch,
            distance,
            yaw_range: None,
            pitch_range: (0.1, 1.4),
            distance_range: (distance * 0.3, distance * 2.0),
            drift: 0.1,
            idle: IDLE_SECONDS,
            phase: None,
        }
    }

    pub fn transform(&self) -> Transform {
        let offset = Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        );
        Transform::from_translation(self.target + offset * self.distance)
            .looking_at(self.target, Vec3::Y)
    }

    fn drift(&mut self, seconds: f32) {
        let Some((low, high)) = self.yaw_range else {
            self.yaw += self.drift * seconds;
            return;
        };
        let (center, amplitude) = ((low + high) / 2.0, (high - low) / 2.0);
        if amplitude <= 0.0 {
            return;
        }
        // Resume the sine from wherever input left the yaw.
        let phase = *self
            .phase
            .get_or_insert_with(|| ((self.yaw - center) / amplitude).clamp(-1.0, 1.0).asin())
            + self.drift / amplitude * seconds;
        self.phase = Some(phase);
        self.yaw = center + amplitude * phase.sin();
    }
}

/// Keeps the orbit centered on this entity (its height stays the camera's own target height).
#[derive(Component, Clone, Copy, Debug)]
pub struct OrbitFollow(pub Entity);

pub fn plugin(app: &mut App) {
    app.add_systems(PostUpdate, orbit.before(TransformSystems::Propagate));
}

fn orbit(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cameras: Query<(&mut OrbitCamera, &mut Transform, Option<&OrbitFollow>)>,
    followed: Query<&Transform, Without<OrbitCamera>>,
) {
    let seconds = time.delta_secs();
    let dragging = buttons.any_pressed([MouseButton::Left, MouseButton::Right]);
    let drag = if dragging { motion.delta } else { Vec2::ZERO };
    let lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_LINE,
    };
    for (mut orbit, mut transform, follow) in &mut cameras {
        if drag != Vec2::ZERO || lines != 0.0 {
            orbit.idle = 0.0;
            orbit.phase = None;
        }
        orbit.yaw -= drag.x * DRAG_SPEED;
        orbit.pitch =
            (orbit.pitch + drag.y * DRAG_SPEED).clamp(orbit.pitch_range.0, orbit.pitch_range.1);
        orbit.distance = (orbit.distance * ZOOM_STEP.powf(lines))
            .clamp(orbit.distance_range.0, orbit.distance_range.1);
        orbit.idle += seconds;
        if orbit.idle >= IDLE_SECONDS {
            orbit.drift(seconds);
        }
        if let Some((low, high)) = orbit.yaw_range {
            orbit.yaw = orbit.yaw.clamp(low, high);
        }
        if let Some(entity) = follow.and_then(|follow| followed.get(follow.0).ok()) {
            let wanted = entity.translation.with_y(orbit.target.y);
            let pull = 1.0 - (-FOLLOW_RATE * seconds).exp();
            orbit.target = orbit.target.lerp(wanted, pull);
        }
        *transform = orbit.transform();
    }
}
