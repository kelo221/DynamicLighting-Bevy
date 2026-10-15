//! Which cube slot each shadowed light keeps from frame to frame, and which faces of its cube
//! must be rendered again. A face keeps its last rendering until its light moves, the casters it
//! draws change, or a caster changes where the face can see it.

use bevy::math::Vec3;

use crate::shared::cube::CUBE_FACE_COUNT;

/// Every face of a cube, one bit per face in WebGPU face order.
pub const ALL_FACES: u8 = (1 << CUBE_FACE_COUNT) - 1;

/// What a cube was rendered from; when it differs, all six faces render again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubePose {
    pub position: Vec3,
    pub radius: f32,
    /// Only meshes without baked data are drawn, because the light uses its baked shadows.
    pub moving_only: bool,
    /// Gaussian blur of the faces, in texels.
    pub blur: f32,
}

/// A shadow caster that appeared, disappeared, moved or changed, as a bounding sphere. A moved
/// caster is reported at both its old and its new bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CasterChange {
    pub center: Vec3,
    pub radius: f32,
    /// The caster has baked data, so cubes drawing only moving meshes don't contain it.
    pub baked: bool,
}

#[derive(Clone, Copy, Debug)]
struct Slot<K> {
    light: K,
    /// `None` until the cube has been rendered in this slot.
    rendered: Option<CubePose>,
}

/// The cube slots of the shadow cubemap array and the light in each.
#[derive(Debug)]
pub struct CubeSlots<K> {
    slots: Vec<Option<Slot<K>>>,
}

impl<K> Default for CubeSlots<K> {
    fn default() -> Self {
        Self { slots: Vec::new() }
    }
}

impl<K: Copy + PartialEq> CubeSlots<K> {
    /// Gives this frame's `lights` their slots: lights that keep a cubemap keep their slot, the
    /// slots of the others are freed, and new lights take the lowest free slots.
    pub fn assign(&mut self, lights: &[K]) {
        for slot in &mut self.slots {
            if slot.is_some_and(|slot| !lights.contains(&slot.light)) {
                *slot = None;
            }
        }
        for &light in lights {
            if self.slot_of(light).is_some() {
                continue;
            }
            let fresh = Some(Slot {
                light,
                rendered: None,
            });
            match self.slots.iter_mut().find(|slot| slot.is_none()) {
                Some(free) => *free = fresh,
                None => self.slots.push(fresh),
            }
        }
        while self.slots.last().is_some_and(Option::is_none) {
            self.slots.pop();
        }
    }

    /// The slot of `light`, if it has one.
    pub fn slot_of(&self, light: K) -> Option<u32> {
        self.slots
            .iter()
            .position(|slot| slot.is_some_and(|slot| slot.light == light))
            .map(|index| index as u32)
    }

    /// Cubes the array must hold: one past the highest slot in use.
    pub fn needed(&self) -> u32 {
        self.slots.len() as u32
    }

    /// The faces of every slot to render this frame, as `(light, faces)` per slot (`None` for free
    /// slots), and records the cubes as rendered. `reset` renders everything: the array was
    /// recreated, or something may have been missing from the last renderings.
    pub fn plan(
        &mut self,
        pose: impl Fn(K) -> CubePose,
        changes: &[CasterChange],
        reset: bool,
    ) -> Vec<Option<(K, u8)>> {
        self.slots
            .iter_mut()
            .map(|slot| {
                let slot = slot.as_mut()?;
                let pose = pose(slot.light);
                let faces = if reset || slot.rendered != Some(pose) {
                    ALL_FACES
                } else {
                    changes
                        .iter()
                        .filter(|change| !(pose.moving_only && change.baked))
                        .fold(0, |faces, change| {
                            faces | faces_touching(pose, change.center, change.radius)
                        })
                };
                slot.rendered = Some(pose);
                Some((slot.light, faces))
            })
            .collect()
    }
}

/// The faces of a cube at `pose` whose 90° view can see part of the sphere at `center`. A sphere
/// around the light touches every face, one out of range none. Conservative: near the edges of
/// a face it may report a face that only just misses the sphere.
pub fn faces_touching(pose: CubePose, center: Vec3, radius: f32) -> u8 {
    let offset = center - pose.position;
    let distance = offset.length();
    if distance > pose.radius + radius {
        return 0;
    }
    if distance <= radius {
        return ALL_FACES;
    }
    // Inside face (axis, sign): `sign * offset[axis] >= |offset[other]|` for both other axes.
    // The sphere reaches those side planes when it is within `radius` of them.
    let slack = radius * core::f32::consts::SQRT_2;
    let mut faces = 0;
    for face in 0..CUBE_FACE_COUNT as usize {
        let (axis, sign) = (face / 2, if face % 2 == 0 { 1.0 } else { -1.0 });
        let along = sign * offset[axis];
        let sees = (0..3)
            .filter(|&other| other != axis)
            .all(|other| along - offset[other].abs() >= -slack);
        if sees {
            faces |= 1 << face;
        }
    }
    faces
}

#[cfg(test)]
mod tests {
    use super::*;

    const POSE: CubePose = CubePose {
        position: Vec3::ZERO,
        radius: 10.0,
        moving_only: false,
        blur: 1.5,
    };

    fn face(index: u32) -> u8 {
        1 << index
    }

    #[test]
    fn lights_keep_their_slots_and_new_lights_fill_the_gaps() {
        let mut slots = CubeSlots::default();
        slots.assign(&['a', 'b', 'c']);
        assert_eq!(slots.slot_of('b'), Some(1));
        slots.assign(&['c', 'a', 'd']);
        assert_eq!(slots.slot_of('a'), Some(0));
        assert_eq!(slots.slot_of('c'), Some(2), "kept although now first");
        assert_eq!(slots.slot_of('d'), Some(1), "took the freed slot");
        assert_eq!(slots.slot_of('b'), None);
        slots.assign(&['a']);
        assert_eq!(slots.needed(), 1, "trailing free slots are dropped");
        slots.assign(&[]);
        assert_eq!(slots.needed(), 0);
    }

    #[test]
    fn new_moved_and_reset_cubes_render_every_face() {
        let mut slots = CubeSlots::default();
        slots.assign(&[1]);
        assert_eq!(slots.plan(|_| POSE, &[], false), [Some((1, ALL_FACES))]);
        assert_eq!(slots.plan(|_| POSE, &[], false), [Some((1, 0))], "cached");
        assert_eq!(slots.plan(|_| POSE, &[], true), [Some((1, ALL_FACES))]);
        let moved = CubePose {
            position: Vec3::X * 0.01,
            ..POSE
        };
        assert_eq!(slots.plan(|_| moved, &[], false), [Some((1, ALL_FACES))]);
        let baked_only = CubePose {
            moving_only: true,
            ..moved
        };
        assert_eq!(
            slots.plan(|_| baked_only, &[], false),
            [Some((1, ALL_FACES))]
        );
        let wider = CubePose {
            blur: 3.0,
            ..baked_only
        };
        assert_eq!(slots.plan(|_| wider, &[], false), [Some((1, ALL_FACES))]);
    }

    #[test]
    fn caster_changes_render_only_the_faces_that_see_them() {
        let mut slots = CubeSlots::default();
        slots.assign(&[1, 2]);
        let far = CubePose {
            position: Vec3::X * 100.0,
            ..POSE
        };
        let pose = |light| if light == 1 { POSE } else { far };
        slots.plan(pose, &[], false);
        let change = CasterChange {
            center: Vec3::new(5.0, 0.0, 0.0),
            radius: 0.5,
            baked: false,
        };
        assert_eq!(
            slots.plan(pose, &[change], false),
            [Some((1, face(0))), Some((2, 0))],
            "the far light can't see it"
        );
        let baked = CasterChange {
            baked: true,
            ..change
        };
        let moving_only = |_| CubePose {
            moving_only: true,
            ..POSE
        };
        slots.assign(&[1]);
        slots.plan(moving_only, &[], false);
        assert_eq!(slots.plan(moving_only, &[baked], false), [Some((1, 0))]);
        assert_eq!(
            slots.plan(moving_only, &[change], false),
            [Some((1, face(0)))]
        );
    }

    #[test]
    fn spheres_touch_the_faces_that_can_see_them() {
        assert_eq!(faces_touching(POSE, Vec3::new(5.0, 0.0, 0.0), 0.5), face(0));
        assert_eq!(
            faces_touching(POSE, Vec3::new(0.0, -5.0, 0.0), 0.5),
            face(3)
        );
        assert_eq!(
            faces_touching(POSE, Vec3::new(0.0, 0.0, -5.0), 0.5),
            face(5)
        );
        assert_eq!(
            faces_touching(POSE, Vec3::new(5.0, 5.0, 0.0), 0.5),
            face(0) | face(2),
            "on the edge between +X and +Y"
        );
        assert_eq!(
            faces_touching(POSE, Vec3::new(4.0, 4.0, 4.0), 0.5),
            face(0) | face(2) | face(4),
            "at a corner"
        );
        assert_eq!(
            faces_touching(POSE, Vec3::new(0.2, 0.0, 0.0), 0.5),
            ALL_FACES
        );
        assert_eq!(faces_touching(POSE, Vec3::new(11.0, 0.0, 0.0), 0.5), 0);
        assert_eq!(
            faces_touching(POSE, Vec3::new(10.4, 0.0, 0.0), 0.5),
            face(0),
            "reaches into range"
        );
    }

    #[test]
    fn every_direction_is_seen_by_the_face_its_axis_points_to() {
        for (direction, expected) in [
            (Vec3::X, 0),
            (Vec3::NEG_X, 1),
            (Vec3::Y, 2),
            (Vec3::NEG_Y, 3),
            (Vec3::Z, 4),
            (Vec3::NEG_Z, 5),
        ] {
            let tilted = (direction * 3.0 + Vec3::splat(0.4)).normalize() * 5.0;
            let faces = faces_touching(POSE, tilted, 0.01);
            assert!(faces & face(expected) != 0, "{direction} → {faces:06b}");
        }
    }
}
