use super::*;
use crate::shared::cube::{CUBE_FACE_COUNT, texel_direction};

/// A distance cube from a light at the origin, baked against a floor at y = −3 and an
/// optional horizontal square blocker `[−1, 1]²` (in x and z, shifted by `offset`) at `height`.
fn cube(height: Option<f32>, offset: f32) -> Vec<f32> {
    let n = CUBE_RESOLUTION;
    (0..CUBE_FACE_COUNT * n * n)
        .map(|index| {
            let face = index / (n * n);
            let direction = texel_direction(face, index % n, (index / n) % n, n);
            let mut hit: f32 = 10.0;
            if direction.y < 0.0 {
                hit = hit.min(-3.0 / direction.y);
            }
            if let Some(height) = height.filter(|_| direction.y < 0.0) {
                let t = height / direction.y;
                let at = direction * t;
                if (at.x - offset).abs() <= 1.0 && at.z.abs() <= 1.0 {
                    hit = hit.min(t);
                }
            }
            hit
        })
        .collect()
}

/// A panel facing down at the origin, or a tube along X when `height` is 0.
fn light(width: f32, height: f32) -> CubeArea {
    CubeArea {
        center: Vec3::ZERO,
        x: Vec3::X,
        y: Vec3::Z,
        half: Vec2::new(width, height) * 0.5,
        radius: 10.0,
    }
}

fn floor_at(x: f32, z: f32) -> Receiver {
    Receiver {
        world: Vec3::new(x, -3.0, z),
        normal: Vec3::Y,
    }
}

fn shadow(light: CubeArea, cube: &[f32], receiver: Receiver) -> f32 {
    cube_area_shadow(light, receiver, |index| cube[index as usize])
}

#[test]
fn open_floors_are_fully_lit() {
    let cube = cube(None, 0.0);
    for x in [-2.0, 0.0, 0.7, 2.5] {
        assert_eq!(shadow(light(1.0, 1.0), &cube, floor_at(x, 0.3)), 1.0, "{x}");
    }
}

#[test]
fn empty_space_past_the_radius_does_not_block() {
    // The cube stores the radius where rays reach nothing within it, and lookups tilted away
    // from a receiver near the edge of the light meet the floor past the radius.
    let radius = 4.0;
    let cube: Vec<f32> = cube(None, 0.0).iter().map(|hit| hit.min(radius)).collect();
    let panel = CubeArea {
        radius,
        ..light(1.5, 1.0)
    };
    for x in [1.5, 2.0, 2.4, 2.6] {
        assert_eq!(shadow(panel, &cube, floor_at(x, 0.4)), 1.0, "{x}");
    }
}

#[test]
fn blockers_cast_an_umbra_and_a_penumbra() {
    // A 2 m blocker 1 m below a 1 m panel; its shadow edge on the floor 3 m down is at x = 3
    // for the panel's center, with the penumbra from x = 2 to 4.
    let cube = cube(Some(-1.0), 0.0);
    let panel = light(1.0, 1.0);
    let at = |x: f32| shadow(panel, &cube, floor_at(x, 0.0));
    assert_eq!(at(0.0), 0.0, "under the blocker");
    assert_eq!(at(4.5), 1.0, "past the penumbra");
    let edge = at(3.0);
    assert!((edge - 0.5).abs() < 0.2, "edge {edge}");
    // The shadow grows lighter outwards across the penumbra.
    let ramp: Vec<f32> = (0..=8).map(|i| at(2.0 + i as f32 * 0.25)).collect();
    assert!(ramp.windows(2).all(|w| w[1] >= w[0] - 0.09), "{ramp:?}");
    assert!(ramp[0] < 0.2 && ramp[8] > 0.8, "{ramp:?}");
}

#[test]
fn penumbras_widen_with_the_light_and_the_blocker_distance() {
    // How many 5 cm floor steps lie strictly inside the penumbra.
    let width = |light: CubeArea, cube: &[f32]| {
        (0..110)
            .map(|i| shadow(light, cube, floor_at(0.5 + i as f32 * 0.05, 0.0)))
            .filter(|&v| v > 0.02 && v < 0.98)
            .count()
    };
    let near = cube(Some(-2.5), 0.0);
    let far = cube(Some(-1.0), 0.0);
    let small = width(light(0.5, 0.5), &far);
    let large = width(light(1.5, 1.5), &far);
    assert!(large > small + 4, "small {small}, large {large}");
    assert!(
        width(light(1.5, 1.5), &near) < large,
        "a blocker near the floor is sharper"
    );
}

#[test]
fn tubes_soften_only_along_their_length() {
    // A tube along X: the shadow edge across x is soft; the one across z only blends about a
    // cube texel (9 cm here).
    let cube = cube(Some(-1.0), 0.0);
    let tube = light(1.5, 0.0);
    let steps = |along_x: bool| {
        (0..60)
            .map(|i| {
                let t = 1.5 + i as f32 * 0.05;
                let receiver = if along_x {
                    floor_at(t, 0.0)
                } else {
                    floor_at(0.0, t)
                };
                shadow(tube, &cube, receiver)
            })
            .filter(|&v| v > 0.02 && v < 0.98)
            .count()
    };
    let (soft, sharp) = (steps(true), steps(false));
    assert!(
        soft >= 4 * sharp && sharp <= 6,
        "soft {soft}, sharp {sharp}"
    );
}

#[test]
fn penumbras_ramp_without_steps() {
    // Across a tube's penumbra in 1 cm steps, no step changes the shadow much more than the
    // mean slope does: the taps' ramps meet instead of leaving one plateau per tap.
    let cube = cube(Some(-1.0), 0.0);
    for half in [0.75, 1.25] {
        let tube = light(half * 2.0, 0.0);
        let ramp: Vec<f32> = (0..300)
            .map(|i| shadow(tube, &cube, floor_at(1.5 + i as f32 * 0.01, 0.0)))
            .collect();
        let inside: Vec<f32> = ramp
            .iter()
            .copied()
            .filter(|v| *v > 0.05 && *v < 0.95)
            .collect();
        let mean = (inside[inside.len() - 1] - inside[0]) / (inside.len() - 1) as f32;
        let steepest = inside.windows(2).map(|w| w[1] - w[0]).fold(0.0, f32::max);
        assert!(
            steepest < 3.0 * mean,
            "half {half}: steepest {steepest}, mean {mean}"
        );
    }
}

#[test]
fn lattice_taps_cover_the_shape_evenly() {
    assert!((GOLDEN_FRACTION - (5.0f32.sqrt() - 1.0) / 2.0).abs() < 1e-6);
    for n in [BLOCKER_TAPS, FILTER_TAPS] {
        let taps: Vec<Vec2> = (0..n).map(|k| lattice_tap(k, n)).collect();
        assert!(taps.iter().all(|tap| tap.abs().max_element() < 1.0));
        // One tap per column, and every half of the height gets some.
        for (k, tap) in taps.iter().enumerate() {
            let column = ((tap.x + 1.0) / 2.0 * n as f32) as usize;
            assert_eq!(column, k);
        }
        let above = taps.iter().filter(|tap| tap.y > 0.0).count();
        assert!(above.abs_diff(n as usize / 2) <= 1, "{above} of {n}");
    }
}

#[test]
fn soft_lookups_agree_with_the_traced_share_of_the_light() {
    // Across the penumbra of a blocker's edge, the lookup is close to the share of a tube's
    // length (or a panel's area) that each floor point sees past the blocker. Filter taps blur
    // by about their spacing, which rounds the penumbra's ends a little.
    for height in [-1.0f32, -1.2, -2.0] {
        let cube = cube(Some(height), 0.0);
        for (light, name) in [(light(1.6, 0.0), "tube"), (light(1.2, 0.8), "panel")] {
            for i in 0..14 {
                let x = 0.8 + i as f32 * 0.25;
                // Light point at offset `t` along x passes the blocker plane at
                // t + (x − t) · |height| / 3; it is blocked when that lies within the blocker.
                let steps = 400;
                let open = (0..steps)
                    .map(|i| ((i as f32 + 0.5) / steps as f32 * 2.0 - 1.0) * light.half.x)
                    .filter(|t| t + (x - t) * -height / 3.0 > 1.0)
                    .count();
                let traced = open as f32 / steps as f32;
                let looked_up = shadow(light, &cube, floor_at(x, 0.0));
                let error = (looked_up - traced).abs();
                assert!(
                    error < 0.075,
                    "{name} under {height} at {x}: {looked_up} vs {traced}"
                );
            }
        }
    }
}
