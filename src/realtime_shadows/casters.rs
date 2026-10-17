//! Which shadow casters each cube face draws: the casters a face's 90° view can see within its
//! light's radius, by their bounding spheres. A cube of a light that uses its baked shadows
//! leaves out baked meshes, whose shadows are in the bake already, so it only holds what the
//! bake can't know about.

use bevy::math::Vec3;

use super::slots::{ALL_FACES, CubePose, faces_touching};

/// A mesh that may cast a shadow into the cubes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caster {
    /// World-space bounding sphere; `None` without bounds (`NoFrustumCulling`), which could be
    /// anywhere.
    pub bounds: Option<(Vec3, f32)>,
    /// The mesh has baked data.
    pub baked: bool,
    /// Visible, and on the default render layer the cubes see.
    pub shown: bool,
}

/// The faces of the cube at `pose` that draw `caster`, one bit per face.
pub fn caster_faces(pose: CubePose, caster: Caster) -> u8 {
    if !caster.shown || (pose.moving_only && caster.baked) {
        return 0;
    }
    match caster.bounds {
        Some((center, radius)) => faces_touching(pose, center, radius),
        None => ALL_FACES,
    }
}

/// A cube with faces rendering this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActiveCube {
    pub slot: u32,
    pub pose: CubePose,
    /// The faces rendering this frame, one bit per face.
    pub faces: u8,
}

/// The casters every rendering face draws, as indices into `casters`, by array layer (six per
/// slot, `layers` in all).
pub fn face_lists(cubes: &[ActiveCube], casters: &[Caster], layers: usize) -> Vec<Vec<u32>> {
    let mut lists = vec![Vec::new(); layers];
    if cubes.is_empty() {
        return lists;
    }
    for (index, &caster) in casters.iter().enumerate() {
        for cube in cubes {
            let faces = caster_faces(cube.pose, caster) & cube.faces;
            for face in (0..6).filter(|face| faces & (1 << face) != 0) {
                if let Some(list) = lists.get_mut(cube.slot as usize * 6 + face) {
                    list.push(index as u32);
                }
            }
        }
    }
    lists
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

    fn at(center: Vec3) -> Caster {
        Caster {
            bounds: Some((center, 0.5)),
            baked: false,
            shown: true,
        }
    }

    #[test]
    fn a_caster_beside_a_light_is_drawn_by_the_face_looking_at_it() {
        assert_eq!(caster_faces(POSE, at(Vec3::new(5.0, 0.0, 0.0))), 1);
        assert_eq!(caster_faces(POSE, at(Vec3::new(0.0, 0.0, -5.0))), 1 << 5);
        assert_eq!(caster_faces(POSE, at(Vec3::new(20.0, 0.0, 0.0))), 0);
    }

    #[test]
    fn baked_casters_stay_out_of_cubes_that_use_the_bake() {
        let baked = Caster {
            baked: true,
            ..at(Vec3::new(5.0, 0.0, 0.0))
        };
        assert_eq!(caster_faces(POSE, baked), 1);
        let moving_only = CubePose {
            moving_only: true,
            ..POSE
        };
        assert_eq!(caster_faces(moving_only, baked), 0);
        assert_eq!(caster_faces(moving_only, at(Vec3::new(5.0, 0.0, 0.0))), 1);
    }

    #[test]
    fn casters_without_bounds_are_in_every_face_and_hidden_ones_in_none() {
        let anywhere = Caster {
            bounds: None,
            ..at(Vec3::ZERO)
        };
        assert_eq!(caster_faces(POSE, anywhere), ALL_FACES);
        let hidden = Caster {
            shown: false,
            ..anywhere
        };
        assert_eq!(caster_faces(POSE, hidden), 0);
        let elsewhere = Caster {
            shown: false,
            ..at(Vec3::new(5.0, 0.0, 0.0))
        };
        assert_eq!(caster_faces(POSE, elsewhere), 0, "another render layer");
    }

    #[test]
    fn only_rendering_faces_get_lists() {
        let far = CubePose {
            position: Vec3::X * 100.0,
            ..POSE
        };
        let cubes = [
            ActiveCube {
                slot: 0,
                pose: POSE,
                faces: 1 | 1 << 5,
            },
            ActiveCube {
                slot: 2,
                pose: far,
                faces: ALL_FACES,
            },
        ];
        let casters = [
            at(Vec3::new(5.0, 0.0, 0.0)),
            at(Vec3::new(0.0, 5.0, 0.0)),
            at(Vec3::new(0.0, 0.0, -5.0)),
            at(Vec3::new(95.0, 0.0, 0.0)),
        ];
        let lists = face_lists(&cubes, &casters, 18);
        assert_eq!(lists[0], [0]);
        assert!(lists[2].is_empty(), "+Y sees caster 1 but doesn't render");
        assert_eq!(lists[5], [2]);
        assert_eq!(lists[12 + 1], [3], "the far cube's -X face");
        let drawn: usize = lists.iter().map(Vec::len).sum();
        assert_eq!(drawn, 3);
        assert!(face_lists(&[], &casters, 18).iter().all(Vec::is_empty));
    }
}
