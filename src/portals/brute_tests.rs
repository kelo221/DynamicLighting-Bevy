//! The portal math against brute force: in a house of three rooms with real walls (boxes with
//! door and window holes), any point of a sphere that a camera can see along a straight,
//! unblocked line inside its frustum must make [`sees`] say yes. This checks the geometry only;
//! the renderer's own reasons to keep lights (shadows that let light through) are tested where
//! lights are picked.

use bevy::math::{Vec3, Vec4Swizzles};

use super::fixtures::{TOLERANCE, camera, door, planes, row};
use super::graph::Graph;
use super::sight::sees;
use super::state::{PortalState, RoomGraph, RoomView};
use super::trace::{TraceCounts, TracedView, trace};

const WALL: f32 = 0.1;
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

/// A wall across X at `x` with a 1 m × 2 m hole centered on `hole` (none if `None`).
fn wall_across_x(x: f32, hole: Option<f32>, blocks: &mut Vec<Block>) {
    let (low, high) = (
        Vec3::new(x - WALL, -WALL, -3.0 - WALL),
        Vec3::new(x + WALL, 3.0 + WALL, 3.0 + WALL),
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

/// The house's walls: the outer shell, the two inner walls with doors, a window at the end.
fn walls() -> Vec<Block> {
    let mut blocks = Vec::new();
    wall_across_x(0.0, None, &mut blocks);
    for (x, z) in DOORS {
        wall_across_x(x, Some(z), &mut blocks);
    }
    wall_across_x(WINDOW.0, Some(WINDOW.1), &mut blocks);
    let span = (
        Vec3::new(-WALL, -WALL, -WALL),
        Vec3::new(18.0 + WALL, 3.0 + WALL, WALL),
    );
    for z in [-3.0, 3.0] {
        let shift = Vec3::new(0.0, 0.0, z);
        blocks.push((span.0 + shift, span.1 + shift));
    }
    for y in [0.0, 3.0] {
        let low = Vec3::new(-WALL, y - WALL, -3.0 - WALL);
        blocks.push((low, Vec3::new(18.0 + WALL, y + WALL, 3.0 + WALL)));
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

#[test]
fn every_point_a_camera_can_see_keeps_its_sphere() {
    let graph = graph();
    let walls = walls();
    let rooms = RoomGraph {
        generation: 1,
        graph: graph.clone(),
        state: PortalState::Ready,
        culprits: Vec::new(),
        rooms: Vec::new(),
        portals: Vec::new(),
    };
    let mut random = Stream(27);
    let (mut framed, mut visible, mut culled, mut unbounded) = (0, 0, 0, 0);
    for _ in 0..300 {
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
        if walls.iter().any(|&block| inside(eye, block)) {
            continue;
        }
        let yaw = random.range(0.0, core::f32::consts::TAU);
        let pitch = random.range(-0.3, 0.3);
        let look = Vec3::new(
            yaw.cos() * pitch.cos(),
            pitch.sin(),
            yaw.sin() * pitch.cos(),
        );
        let frustum = camera(eye, eye + look);
        let mut traced = TracedView::default();
        let bounded = trace(
            &graph,
            eye,
            &planes(&frustum),
            16,
            &mut traced,
            &mut TraceCounts::default(),
        );
        unbounded += u32::from(!bounded);
        let view = RoomView {
            generation: 1,
            bounded,
            traced,
        };
        let in_frustum = |point: Vec3| {
            frustum.half_spaces[..5]
                .iter()
                .all(|half| half.normal_d().xyz().dot(point) + half.d() >= 0.0)
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
            framed += 1;
            culled += u32::from(!kept);
            let sighted = (0..64)
                .map(|_| center + random.in_ball() * radius)
                .find(|&point| {
                    in_frustum(point)
                        && !walls
                            .iter()
                            .any(|&block| inside(point, block) || blocked(eye, point, block))
                });
            if let Some(point) = sighted {
                visible += 1;
                assert!(
                    kept,
                    "eye {eye} looking {look}: {point} of the sphere at {center} r {radius} is in sight"
                );
            }
        }
    }
    assert_eq!(unbounded, 0);
    // Neither side is vacuous: many spheres are in sight, and many in the frustum are culled.
    let counts = format!("{framed} framed, {visible} in sight, {culled} culled");
    assert!(framed > 2000 && visible > 500, "{counts}");
    assert!(culled * 4 > framed, "{counts}");
}
