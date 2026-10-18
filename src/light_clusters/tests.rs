//! The cluster lists against brute force: every light whose sphere holds a point is listed in
//! the point's cell, whatever the camera.

use bevy::camera::{CameraProjection, OrthographicProjection, PerspectiveProjection};
use bevy::math::{Mat4, UVec3, Vec3};
use bevy::prelude::Transform;

use super::assign::{ClusterLists, GridOff, MAX_ENTRIES, build, light_sphere};
use super::gpu::GPU_CLUSTER_HEADER_SIZE;
use super::grid::{Cell, GRID, Grid};
use crate::effects::EffectRng;
use crate::light::{LightType, RealtimeViewer};
use crate::shared::{ChannelFlags, GpuLight};

fn viewer(projection: Mat4) -> RealtimeViewer {
    let transform =
        Transform::from_xyz(2.0, 6.0, 14.0).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y);
    RealtimeViewer {
        view_from_world: transform.to_matrix().inverse(),
        clip_from_view: projection,
    }
}

fn perspective() -> RealtimeViewer {
    viewer(
        PerspectiveProjection {
            aspect_ratio: 16.0 / 9.0,
            ..Default::default()
        }
        .get_clip_from_view(),
    )
}

fn orthographic() -> RealtimeViewer {
    let mut projection = OrthographicProjection::default_3d();
    projection.update(32.0, 18.0);
    viewer(projection.get_clip_from_view())
}

fn light(kind: LightType, position: Vec3, radius: f32, half: (f32, f32)) -> GpuLight {
    let channel = ChannelFlags {
        realtime: true,
        light_type: kind as u32,
        ..Default::default()
    };
    let extent = if kind.is_area() {
        (half.0 * half.0 + half.1 * half.1).sqrt()
    } else {
        0.0
    };
    GpuLight {
        position: position.to_array(),
        radius_sqr: (radius - extent).powi(2),
        channel: channel.pack(),
        gp1: half.0,
        gp2: half.1,
        ..Default::default()
    }
}

fn random_lights(rng: &mut EffectRng, count: usize) -> Vec<GpuLight> {
    let mut range = |low: f32, high: f32| low + (high - low) * rng.next_f32();
    (0..count)
        .map(|index| {
            let position = Vec3::new(range(-14.0, 14.0), range(-1.0, 4.0), range(-14.0, 10.0));
            let radius = range(0.3, 4.0);
            match index % 4 {
                0 => light(
                    LightType::Rectangle,
                    position,
                    radius,
                    (radius * 0.3, radius * 0.2),
                ),
                1 => light(LightType::Tube, position, radius, (radius * 0.4, 0.0)),
                2 => light(LightType::Spot, position, radius, (0.0, 0.0)),
                _ => light(LightType::Point, position, radius, (0.0, 0.0)),
            }
        })
        .collect()
}

/// Checks `samples` points in and around the lights against brute force, as the shader looks
/// them up.
fn check_every_point(viewer: RealtimeViewer, lights: &[GpuLight], samples: usize) -> usize {
    let lists = build(Some(viewer), lights, 0).expect("the grid is on");
    let spheres: Vec<_> = lights.iter().filter_map(light_sphere).collect();
    let grid = Grid::new(viewer, spheres.iter().copied()).unwrap();
    let mut rng = EffectRng::new(7, 11);
    let mut checked = 0;
    for sample in 0..samples {
        let sphere = spheres[sample % spheres.len()];
        // Points inside the sphere, half of them on its surface, where cells are thinnest.
        let direction = loop {
            let v = Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32()) * 2.0 - 1.0;
            if v.length_squared() > 1.0e-4 && v.length_squared() <= 1.0 {
                break v.normalize();
            }
        };
        let distance = if sample % 2 == 0 {
            0.9999
        } else {
            rng.next_f32().cbrt()
        };
        let point = sphere.center + direction * sphere.radius * distance;
        let listed: Vec<u32> = match grid.cell_at(point) {
            Cell::Outside => continue,
            Cell::Empty => Vec::new(),
            Cell::At(cell) => lists.lights_in(cell).collect(),
        };
        for (index, other) in lights.iter().enumerate() {
            let Some(other) = light_sphere(other) else {
                continue;
            };
            if point.distance(other.center) <= other.radius {
                assert!(
                    listed.contains(&(index as u32)),
                    "light {index} reaches {point} but its cell doesn't list it"
                );
            }
        }
        checked += 1;
    }
    checked
}

#[test]
fn every_light_reaching_a_point_is_listed_in_its_cell() {
    let mut rng = EffectRng::new(3, 5);
    let lights = random_lights(&mut rng, 200);
    let checked = check_every_point(perspective(), &lights, 20_000);
    assert!(checked > 5_000, "only {checked} points were in view");
}

#[test]
fn orthographic_cameras_list_every_light_too() {
    let mut rng = EffectRng::new(9, 1);
    let lights = random_lights(&mut rng, 120);
    let checked = check_every_point(orthographic(), &lights, 10_000);
    assert!(checked > 2_000, "only {checked} points were in view");
}

#[test]
fn cells_list_far_fewer_lights_than_there_are() {
    let mut rng = EffectRng::new(3, 5);
    let lights = random_lights(&mut rng, 200);
    let lists = build(Some(perspective()), &lights, 0).unwrap();
    let cells = GRID.element_product() as usize;
    let mean = lists.entries as f32 / cells as f32;
    assert!(mean < 20.0, "{mean} lights per cell on average");
}

#[test]
fn cells_beside_a_sphere_in_its_block_dont_list_it() {
    let mut rng = EffectRng::new(3, 5);
    let lights = random_lights(&mut rng, 200);
    let lists = build(Some(perspective()), &lights, 0).unwrap();
    let spheres: Vec<_> = lights.iter().filter_map(light_sphere).collect();
    let grid = Grid::new(perspective(), spheres.iter().copied()).unwrap();
    let blocks: usize = spheres
        .iter()
        .filter_map(|&sphere| grid.cells_touched(sphere))
        .map(|(low, high)| (high - low + 1).element_product() as usize)
        .sum();
    assert!(
        lists.entries * 10 < blocks * 9,
        "{} entries of {blocks} block cells",
        lists.entries
    );
}

#[test]
fn every_cell_bound_holds_the_points_of_its_cell() {
    for viewer in [perspective(), orthographic()] {
        let lights = [light(LightType::Point, Vec3::ZERO, 12.0, (0.0, 0.0))];
        let grid = Grid::new(viewer, lights.iter().filter_map(light_sphere)).unwrap();
        let bounds = grid.cell_bounds();
        let mut rng = EffectRng::new(1, 2);
        for _ in 0..20_000 {
            let v = Vec3::new(rng.next_f32(), rng.next_f32(), rng.next_f32()) * 2.0 - 1.0;
            let point = v * 12.0;
            if let Cell::At(cell) = grid.cell_at(point) {
                assert!(
                    bounds.reaches(cell, grid.view_point(point), 0.0),
                    "{point} is outside the bounds of its cell {cell}"
                );
            }
        }
    }
}

#[test]
fn cells_list_exactly_the_lights_whose_bounds_test_passes() {
    let mut rng = EffectRng::new(4, 8);
    let lights = random_lights(&mut rng, 150);
    let lists = build(Some(perspective()), &lights, 0).unwrap();
    let grid = Grid::new(perspective(), lights.iter().filter_map(light_sphere)).unwrap();
    let bounds = grid.cell_bounds();
    for cell in 0..GRID.element_product() {
        let at = UVec3::new(
            cell % GRID.x,
            cell / GRID.x % GRID.y,
            cell / (GRID.x * GRID.y),
        );
        let expected: Vec<u32> = (0..lights.len() as u32)
            .filter(|&index| {
                let Some(sphere) = light_sphere(&lights[index as usize]) else {
                    return false;
                };
                let Some((low, high)) = grid.cells_touched(sphere) else {
                    return false;
                };
                at.cmpge(low).all()
                    && at.cmple(high).all()
                    && bounds.reaches(cell, grid.view_point(sphere.center), sphere.radius)
            })
            .collect();
        assert_eq!(
            lists.lights_in(cell).collect::<Vec<_>>(),
            expected,
            "cell {cell}"
        );
    }
}

#[test]
fn points_behind_or_beside_the_camera_visit_every_light() {
    let lights = [light(LightType::Point, Vec3::ZERO, 3.0, (0.0, 0.0))];
    let viewer = perspective();
    let grid = Grid::new(viewer, lights.iter().filter_map(light_sphere)).unwrap();
    assert_eq!(grid.cell_at(Vec3::new(4.0, 12.0, 30.0)), Cell::Outside);
    assert_eq!(grid.cell_at(Vec3::new(200.0, 0.0, 0.0)), Cell::Outside);
    // Straight ahead but past every light.
    assert_eq!(grid.cell_at(Vec3::new(-2.0, -5.0, -14.0)), Cell::Empty);
    assert!(matches!(grid.cell_at(Vec3::ZERO), Cell::At(_)));
}

#[test]
fn depth_slices_get_deeper_with_distance() {
    let lights = [light(
        LightType::Point,
        Vec3::new(0.0, 0.5, -30.0),
        4.0,
        (0.0, 0.0),
    )];
    let viewer = perspective();
    let grid = Grid::new(viewer, lights.iter().filter_map(light_sphere)).unwrap();
    let eye = Vec3::new(2.0, 6.0, 14.0);
    let ahead = (Vec3::new(0.0, 0.5, 0.0) - eye).normalize();
    let slices: Vec<u32> = [0.5, 2.0, 4.0, 8.0, 16.0, 32.0, 46.0]
        .iter()
        .map(|&distance| match grid.cell_at(eye + ahead * distance) {
            Cell::At(cell) => cell / (GRID.x * GRID.y),
            other => panic!("{other:?} at {distance}"),
        })
        .collect();
    assert_eq!(slices[0], 0, "the first slice holds what is right in front");
    assert!(
        slices.windows(2).all(|pair| pair[0] < pair[1]),
        "{slices:?}"
    );
    // Equal steps in depth cross fewer slices further out.
    assert!(slices[6] - slices[5] < slices[3] - slices[1], "{slices:?}");
}

#[test]
fn area_lights_reach_past_their_shape() {
    let panel = light(LightType::Rectangle, Vec3::ZERO, 5.0, (1.5, 2.0));
    let sphere = light_sphere(&panel).unwrap();
    assert!((sphere.radius - 5.0).abs() < 1.0e-5);
    let tube = light(LightType::Tube, Vec3::ZERO, 3.0, (1.0, 0.0));
    assert!((light_sphere(&tube).unwrap().radius - 3.0).abs() < 1.0e-5);
    assert_eq!(light_sphere(&GpuLight::DISABLED), None);
}

#[test]
fn suns_are_in_no_cell() {
    let sun = GpuLight {
        radius_sqr: f32::MAX,
        ..light(LightType::Sun, Vec3::ZERO, 1.0, (1.0, 0.0))
    };
    assert_eq!(light_sphere(&sun), None);
}

#[test]
fn the_grid_stays_off_when_it_cant_help() {
    let lights = vec![light(LightType::Point, Vec3::ZERO, 2.0, (0.0, 0.0)); 8];
    assert_eq!(build(None, &lights, 0), Err(GridOff::NoCamera));
    assert_eq!(
        build(Some(perspective()), &lights, 9),
        Err(GridOff::FewLights)
    );
    assert_eq!(build(Some(perspective()), &[], 0), Err(GridOff::FewLights));
    assert!(build(Some(perspective()), &lights, 8).is_ok());
    let behind = vec![light(LightType::Point, Vec3::new(2.0, 6.0, 40.0), 2.0, (0.0, 0.0)); 8];
    assert_eq!(
        build(Some(perspective()), &behind, 0),
        Err(GridOff::NothingInFront)
    );
    let disabled = vec![GpuLight::DISABLED; 8];
    assert_eq!(
        build(Some(perspective()), &disabled, 0),
        Err(GridOff::NothingInFront)
    );
}

#[test]
fn too_many_entries_turn_the_grid_off() {
    let cells = GRID.element_product() as usize;
    let count = MAX_ENTRIES / cells + 1;
    let huge = vec![light(LightType::Point, Vec3::ZERO, 500.0, (0.0, 0.0)); count];
    assert_eq!(
        build(Some(perspective()), &huge, 0),
        Err(GridOff::TooManyEntries)
    );
}

#[test]
fn the_buffer_is_the_header_then_the_words() {
    let off = ClusterLists::default().to_bytes();
    assert_eq!(off.len(), GPU_CLUSTER_HEADER_SIZE + 16);
    assert!(off.iter().all(|&byte| byte == 0), "an off grid is all zero");

    let lights = vec![light(LightType::Point, Vec3::ZERO, 2.0, (0.0, 0.0)); 3];
    let lists = build(Some(perspective()), &lights, 0).unwrap();
    let bytes = lists.to_bytes();
    let words_end = GPU_CLUSTER_HEADER_SIZE + 4 * lists.words.len();
    assert_eq!(bytes.len(), words_end.next_multiple_of(16));
    assert_eq!(
        &bytes[GPU_CLUSTER_HEADER_SIZE..words_end],
        bytemuck::cast_slice::<u32, u8>(&lists.words)
    );
    assert!(bytes[words_end..].iter().all(|&byte| byte == 0));
    let cells = GRID.element_product();
    assert_eq!(lists.header.entry_base, 2 * cells);
    assert_eq!(lists.header.dims, GRID.to_array());
    assert_eq!(
        lists.words.len(),
        (2 * cells) as usize + lists.entries.div_ceil(2)
    );
    assert_eq!(build(Some(perspective()), &lights, 0).unwrap(), lists);
}
