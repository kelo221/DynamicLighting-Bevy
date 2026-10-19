//! The portal math against brute force: in a house of three rooms with real walls (boxes with
//! door and window holes), any point of a sphere that a camera can see along a straight,
//! unblocked line inside its frustum must make [`sees`] say yes. As in the renderer, nothing
//! nearer than the near plane blocks the line, so a camera pressed against a thin wall sees past
//! it. This checks the geometry only; the renderer's own reasons to keep lights (shadows that let
//! light through) are tested where lights are picked.

use bevy::camera::PerspectiveProjection;
use bevy::math::{Vec3, Vec4};

use super::cell::distance;

use super::fixtures::{TOLERANCE, camera, door, near, planes, row};
use super::graph::Graph;
use super::sight::sees;
use super::state::{PortalState, RoomGraph, RoomView};
use super::trace::{TraceCounts, TracedView, trace};

/// Interior doors at `x = 6, 12` (centered on these `z`), a window at `x = 18`.
const DOORS: [(f32, f32); 2] = [(6.0, 0.0), (12.0, 1.5)];
const WINDOW: (f32, f32) = (18.0, -1.0);

/// A stateless random stream (SplitMix64), for repeatable samples.
struct Stream(u64);

impl Stream {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }

    fn in_ball(&mut self) -> Vec3 {
        loop {
            let point = Vec3::new(
                self.range(-1.0, 1.0),
                self.range(-1.0, 1.0),
                self.range(-1.0, 1.0),
            );
            if point.length_squared() <= 1.0 {
                return point;
            }
        }
    }
}

/// An axis-aligned box of wall, as (min, max).
type Block = (Vec3, Vec3);

/// A wall `2 · half` thick across X at `x` with a 1 m × 2 m hole centered on `hole`.
fn wall_across_x(x: f32, half: f32, hole: Option<f32>, blocks: &mut Vec<Block>) {
    let (low, high) = (
        Vec3::new(x - half, -half, -3.0 - half),
        Vec3::new(x + half, 3.0 + half, 3.0 + half),
    );
    let Some(z) = hole else {
        blocks.push((low, high));
        return;
    };
    blocks.push((low, Vec3::new(high.x, high.y, z - 0.5)));
    blocks.push((Vec3::new(low.x, low.y, z + 0.5), high));
    blocks.push((
        Vec3::new(low.x, 2.0, z - 0.5),
        Vec3::new(high.x, high.y, z + 0.5),
    ));
}

/// The house's walls, `2 · half` thick: the outer shell, the two inner walls with doors, a
/// window at the end.
fn walls(half: f32) -> Vec<Block> {
    let mut blocks = Vec::new();
    wall_across_x(0.0, half, None, &mut blocks);
    for (x, z) in DOORS {
        wall_across_x(x, half, Some(z), &mut blocks);
    }
    wall_across_x(WINDOW.0, half, Some(WINDOW.1), &mut blocks);
    let span = (
        Vec3::new(-half, -half, -half),
        Vec3::new(18.0 + half, 3.0 + half, half),
    );
    for z in [-3.0, 3.0] {
        let shift = Vec3::new(0.0, 0.0, z);
        blocks.push((span.0 + shift, span.1 + shift));
    }
    for y in [0.0, 3.0] {
        let low = Vec3::new(-half, y - half, -3.0 - half);
        blocks.push((low, Vec3::new(18.0 + half, y + half, 3.0 + half)));
    }
    blocks
}

fn graph() -> Graph {
    let (rooms, mut portals) = row(3, &[DOORS[0].1, DOORS[1].1]);
    portals.push(door(WINDOW.0, WINDOW.1));
    Graph::build(&rooms, &portals, TOLERANCE).unwrap()
}

fn inside(point: Vec3, (low, high): Block) -> bool {
    point.cmpge(low).all() && point.cmple(high).all()
}

/// Whether the segment from `a` to `b` passes through the box.
fn blocked(a: Vec3, b: Vec3, (low, high): Block) -> bool {
    let direction = b - a;
    let (mut near, mut far) = (0.0_f32, 1.0_f32);
    for axis in 0..3 {
        let (start, step) = (a[axis], direction[axis]);
        if step.abs() < 1e-9 {
            if start < low[axis] || start > high[axis] {
                return false;
            }
            continue;
        }
        let (t0, t1) = ((low[axis] - start) / step, (high[axis] - start) / step);
        near = near.max(t0.min(t1));
        far = far.min(t0.max(t1));
        if near > far {
            return false;
        }
    }
    true
}

/// The plane the renderer clips at, in front of the eye. Bevy's frustum puts its near half-space
/// as far behind the eye, so it can't stand in for this one.
fn near_plane(eye: Vec3, look: Vec3) -> Vec4 {
    let normal = look.normalize();
    normal.extend(-normal.dot(eye) - PerspectiveProjection::default().near)
}

/// Where the line from `eye` to `point` (past the near plane) crosses the near plane.
fn on_near_plane(plane: Vec4, eye: Vec3, point: Vec3) -> Vec3 {
    let (from, to) = (distance(plane, eye), distance(plane, point));
    eye + (point - eye) * (-from / (to - from))
}

/// A camera looking anywhere from anywhere: a fifth of them outside the house.
fn anywhere(random: &mut Stream) -> (Vec3, Vec3) {
    let eye = if random.next() < 0.2 {
        Vec3::new(
            random.range(19.0, 24.0),
            random.range(0.5, 2.5),
            random.range(-4.0, 4.0),
        )
    } else {
        Vec3::new(
            random.range(0.3, 17.7),
            random.range(0.3, 2.7),
            random.range(-2.7, 2.7),
        )
    };
    let yaw = random.range(0.0, core::f32::consts::TAU);
    (eye, look(yaw, random.range(-0.3, 0.3)))
}

/// A camera up to 9 cm from a wall's face (on either side of it), looking towards the wall.
fn against_a_wall(random: &mut Stream, half: f32) -> (Vec3, Vec3) {
    let side = if random.next() < 0.5 { -1.0 } else { 1.0 };
    let gap = half + random.range(0.005, 0.09);
    let (y, z) = (random.range(0.3, 2.7), random.range(-2.7, 2.7));
    let turn = random.range(-1.0, 1.0);
    let pitch = random.range(-0.3, 0.3);
    if random.next() < 0.7 {
        let x = [0.0, 6.0, 12.0, 18.0][(random.next() * 4.0) as usize % 4];
        let yaw = if side > 0.0 {
            core::f32::consts::PI
        } else {
            0.0
        };
        (Vec3::new(x + side * gap, y, z), look(yaw + turn, pitch))
    } else {
        let wall = if random.next() < 0.5 { -3.0 } else { 3.0 };
        let yaw = -side * core::f32::consts::FRAC_PI_2;
        let x = random.range(0.3, 17.7);
        (Vec3::new(x, y, wall + side * gap), look(yaw + turn, pitch))
    }
}

fn look(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(
        yaw.cos() * pitch.cos(),
        pitch.sin(),
        yaw.sin() * pitch.cos(),
    )
}

/// What one run saw: spheres in a frustum, in sight, culled, and in sight only because the near
/// plane clips a wall away.
#[derive(Default)]
struct Tally {
    framed: u32,
    visible: u32,
    culled: u32,
    past_walls: u32,
}

/// Checks `cameras` cameras from `place` in a house with walls `2 · half` thick.
fn check(half: f32, seed: u64, cameras: u32, place: impl Fn(&mut Stream) -> (Vec3, Vec3)) -> Tally {
    let graph = graph();
    let walls = walls(half);
    let rooms = RoomGraph {
        generation: 1,
        graph: graph.clone(),
        state: PortalState::Ready,
        culprits: Vec::new(),
        rooms: Vec::new(),
        portals: Vec::new(),
    };
    let mut random = Stream(seed);
    let mut tally = Tally::default();
    for _ in 0..cameras {
        let (eye, look) = place(&mut random);
        if walls.iter().any(|&block| inside(eye, block)) {
            continue;
        }
        let frustum = camera(eye, eye + look);
        let mut traced = TracedView::default();
        let bounded = trace(
            &graph,
            eye,
            &near(eye, eye + look),
            &planes(&frustum),
            16,
            &mut traced,
            &mut TraceCounts::default(),
        );
        assert!(bounded, "eye {eye} looking {look}: unbounded");
        let view = RoomView {
            generation: 1,
            bounded,
            traced,
        };
        let clip = near_plane(eye, look);
        let in_frustum = |point: Vec3| {
            distance(clip, point) >= 0.0
                && frustum.half_spaces[..4]
                    .iter()
                    .all(|half| distance(half.normal_d(), point) >= 0.0)
        };
        for _ in 0..40 {
            let center = Vec3::new(
                random.range(-2.0, 26.0),
                random.range(0.0, 3.0),
                random.range(-4.0, 4.0),
            );
            let radius = random.range(0.1, 2.0);
            let kept = sees(&rooms, [(&frustum, Some(&view))], center, radius);
            if !crate::shared::sees_sphere(&frustum, center, radius) {
                continue;
            }
            tally.framed += 1;
            tally.culled += u32::from(!kept);
            let sighted = (0..64)
                .map(|_| center + random.in_ball() * radius)
                .find(|&point| {
                    let from = on_near_plane(clip, eye, point);
                    in_frustum(point)
                        && !walls
                            .iter()
                            .any(|&block| inside(point, block) || blocked(from, point, block))
                });
            if let Some(point) = sighted {
                tally.visible += 1;
                let hidden = walls.iter().any(|&block| blocked(eye, point, block));
                tally.past_walls += u32::from(hidden);
                assert!(
                    kept,
                    "eye {eye} looking {look}: {point} of the sphere at {center} r {radius} is in sight"
                );
            }
        }
    }
    tally
}

#[test]
fn every_point_a_camera_can_see_keeps_its_sphere() {
    let tally = check(0.1, 27, 300, anywhere);
    // Neither side is vacuous: many spheres are in sight, and many in the frustum are culled.
    let Tally {
        framed,
        visible,
        culled,
        past_walls,
    } = tally;
    let counts = format!("{framed} framed, {visible} in sight, {culled} culled");
    assert!(framed > 2000 && visible > 500, "{counts}");
    assert!(culled * 4 > framed, "{counts}");
    // 20 cm walls are thicker than the 10 cm to the near plane.
    assert_eq!(past_walls, 0);
}

#[test]
fn a_camera_pressed_against_a_thin_wall_keeps_what_it_sees_past_it() {
    let tally = check(0.02, 41, 300, |random| against_a_wall(random, 0.02));
    let Tally {
        framed,
        visible,
        past_walls,
        ..
    } = tally;
    let counts = format!("{framed} framed, {visible} in sight, {past_walls} past a wall");
    assert!(visible > 500 && past_walls > 400, "{counts}");
}
