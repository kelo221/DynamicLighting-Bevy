//! The sun's cascades: where they sit, what they hold and draw, when they render again, and how
//! the mirror of `sun_vsm.wesl` shadows a receiver with them.

use bevy::camera::CameraProjection;
use bevy::math::Vec4Swizzles;
use bevy::prelude::*;

use super::cascades::{CascadePose, CascadeViewer, fit_cascades, plan_cascades, split_depths};
use super::casters::Caster;
use super::slots::CasterChange;
use super::sun_mirror::{cascade_point, sun_shadow};
use crate::shared::{GpuSunShadow, SUN_CASCADE_COUNT};

const DISTANCE: f32 = 50.0;

fn camera() -> Transform {
    Transform::from_xyz(0.0, 6.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y)
}

fn viewer(transform: Transform) -> CascadeViewer {
    let projection = PerspectiveProjection {
        fov: 0.8,
        aspect_ratio: 16.0 / 9.0,
        near: 0.1,
        ..default()
    };
    CascadeViewer {
        world_from_view: transform.to_matrix(),
        clip_from_view: projection.get_clip_from_view(),
    }
}

fn to_sun() -> Vec3 {
    Vec3::new(0.3, 1.0, 0.2).normalize()
}

fn cascades(to_sun: Vec3, moving_only: bool, casters: &[(Vec3, f32)]) -> [CascadePose; 3] {
    fit_cascades(&viewer(camera()), to_sun, DISTANCE, moving_only, casters)
}

/// One bit per cascade that `touched` holds.
fn mask(poses: &[CascadePose; 3], touched: impl Fn(&CascadePose) -> bool) -> u8 {
    (0..SUN_CASCADE_COUNT).fold(0, |bits, index| {
        bits | (u8::from(touched(&poses[index])) << index)
    })
}

#[test]
fn splits_run_from_the_near_plane_to_the_shadow_distance() {
    let depths = split_depths(0.1, DISTANCE);
    assert_eq!(depths[0], 0.1);
    assert!((depths[SUN_CASCADE_COUNT] - DISTANCE).abs() < 1e-3);
    assert!(depths.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(
        depths[1] < DISTANCE / 6.0,
        "near cascades are the small ones"
    );
}

#[test]
fn each_cascade_holds_its_slice_of_the_view_inside_its_rim() {
    let viewer = viewer(camera());
    let depths = split_depths(viewer.near(), DISTANCE);
    let poses = cascades(to_sun(), false, &[]);
    for (index, pose) in poses.iter().enumerate() {
        let gpu = pose.gpu();
        for corner in viewer.slice(depths[index], depths[index + 1]) {
            assert!(pose.touches(corner, 0.0), "cascade {index}");
            let at = cascade_point(&gpu, corner, Vec3::ZERO);
            assert!(at.rim < 0.98, "cascade {index}: rim {}", at.rim);
            assert!((0.0..1.0).contains(&at.depth), "cascade {index}");
        }
    }
    assert!(poses[0].half < poses[1].half && poses[1].half < poses[2].half);
}

#[test]
fn the_lit_shader_finds_texels_where_the_cascade_view_drew_them() {
    let viewer = viewer(camera());
    let depths = split_depths(viewer.near(), DISTANCE);
    for (index, pose) in cascades(to_sun(), false, &[]).iter().enumerate() {
        let (view, projection) = pose.view();
        let clip_from_world = projection * view.to_matrix().inverse();
        let gpu = pose.gpu();
        for point in viewer.slice(depths[index], depths[index + 1]) {
            let clip = clip_from_world * point.extend(1.0);
            let ndc = clip.xyz() / clip.w;
            let uv = Vec2::new(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
            let at = cascade_point(&gpu, point, Vec3::ZERO);
            assert!(at.uv.abs_diff_eq(uv, 1e-4), "{} vs {uv}", at.uv);
            // The moments pass stores 1 - depth: 0 at the sunward end.
            assert!((at.depth - (1.0 - ndc.z)).abs() < 1e-4);
        }
    }
}

#[test]
fn small_camera_moves_keep_the_cascades_in_place() {
    let before = cascades(to_sun(), false, &[]);
    assert_eq!(before, cascades(to_sun(), false, &[]));
    let moved = camera().with_translation(camera().translation + Vec3::new(0.003, 0.0, -0.002));
    let turned = camera().with_rotation(camera().rotation * Quat::from_rotation_y(0.002));
    for transform in [moved, turned] {
        let after = fit_cascades(&viewer(transform), to_sun(), DISTANCE, false, &[]);
        for (a, b) in before.iter().zip(&after) {
            assert_eq!(a.half, b.half, "the sizes don't change");
            assert!((a.center - b.center).abs().max_element() <= 1);
        }
    }
}

#[test]
fn cascades_reach_up_to_the_casters_over_them() {
    let open = cascades(Vec3::Y, false, &[]);
    let caster = (Vec3::new(0.0, 80.0, 0.0), 2.0);
    let poses = cascades(Vec3::Y, false, &[caster]);
    let over = mask(&poses, |pose| pose.touches(caster.0, caster.1));
    assert_ne!(over, 0, "the caster stands over the view");
    for index in 0..SUN_CASCADE_COUNT {
        let top = cascade_point(
            &poses[index].gpu(),
            caster.0 + Vec3::Y * caster.1,
            Vec3::ZERO,
        );
        if over & (1 << index) != 0 {
            assert!(top.depth >= 0.0, "cascade {index}");
            assert!(poses[index].range() > open[index].range());
        } else {
            assert_eq!(poses[index], open[index], "cascade {index}");
        }
    }
}

#[test]
fn cascades_render_again_only_when_needed() {
    let poses = cascades(to_sun(), true, &[]);
    let mut rendered = [None; SUN_CASCADE_COUNT];
    let mut plan = |poses: &[CascadePose; 3], changes: &[CasterChange], missed, reset| {
        plan_cascades(&mut rendered, poses, changes, missed, reset)
    };
    assert_eq!(plan(&poses, &[], 0, false), 0b111, "the first rendering");
    assert_eq!(plan(&poses, &[], 0, false), 0, "nothing changed");

    let center = camera().translation + camera().forward() * 1.0;
    let change = CasterChange {
        center,
        radius: 0.1,
        baked: false,
    };
    let touched = mask(&poses, |pose| pose.touches(center, 0.1));
    assert_ne!(touched & 1, 0, "the nearest cascade holds it");
    assert_eq!(plan(&poses, &[change], 0, false), touched);
    let baked = CasterChange {
        baked: true,
        ..change
    };
    assert_eq!(plan(&poses, &[baked], 0, false), 0, "the bake has it");
    assert_eq!(plan(&poses, &[], 0b100, false), 0b100, "it missed a caster");
    assert_eq!(plan(&poses, &[], 0, true), 0b111, "reset");
    let mut moved = poses;
    moved[1].center.x += 1;
    assert_eq!(
        plan(&moved, &[], 0, false),
        0b010,
        "it snapped to a new place"
    );
}

#[test]
fn cascades_draw_what_can_shadow_them() {
    let pose = cascades(to_sun(), true, &[])[0];
    let point = camera().translation + camera().forward() * 1.0;
    let caster = |bounds, baked, shown| Caster {
        bounds,
        baked,
        shown,
    };
    let inside = Some((point, 0.5));
    assert!(pose.draws(caster(inside, false, true)));
    assert!(!pose.draws(caster(inside, true, true)), "the bake has it");
    assert!(!pose.draws(caster(inside, false, false)), "hidden");
    assert!(pose.draws(caster(None, false, true)), "could be anywhere");
    let aside = Some((Vec3::new(500.0, 0.0, 500.0), 1.0));
    assert!(
        !pose.draws(caster(aside, false, true)),
        "beside the cascade"
    );
    let below = Some((point - to_sun() * 300.0, 1.0));
    assert!(!pose.draws(caster(below, false, true)), "past its far end");
}

/// The cascades of a sun overhead over the view of `camera`, and their filtered maps with a box
/// whose top is at `height` over x and z in -0.5..0.5, in the cascades that draw it.
fn box_over(camera: Transform, height: f32) -> (GpuSunShadow, impl Fn(usize, Vec2) -> Vec2) {
    let bounds = (Vec3::new(0.0, height - 0.5, 0.0), 0.9);
    let poses = fit_cascades(&viewer(camera), Vec3::Y, DISTANCE, false, &[bounds]);
    let shadow = GpuSunShadow {
        cascades: poses.map(|pose| pose.gpu()),
        count: SUN_CASCADE_COUNT as u32,
        ..default()
    };
    let gpu = shadow.cascades;
    let draws = poses.map(|pose| pose.touches(bounds.0, bounds.1));
    let sample = move |index: usize, uv: Vec2| {
        let at = |x: f32, z: f32| cascade_point(&gpu[index], Vec3::new(x, height, z), Vec3::ZERO);
        let (a, b) = (at(-0.5, -0.5), at(0.5, 0.5));
        let (low, high) = (a.uv.min(b.uv), a.uv.max(b.uv));
        if draws[index] && uv.cmpge(low).all() && uv.cmple(high).all() {
            let depth = a.depth;
            Vec2::new(depth, depth * depth)
        } else {
            Vec2::ONE
        }
    };
    (shadow, sample)
}

fn box_scene() -> (GpuSunShadow, impl Fn(usize, Vec2) -> Vec2) {
    box_over(camera(), 1.0)
}

#[test]
fn the_box_shadows_the_ground_under_it() {
    let (shadow, sample) = box_scene();
    let lit = |point: Vec3, static_receiver| {
        sun_shadow(&shadow, point, Vec3::Y, 1.0, static_receiver, &sample)
    };
    assert_eq!(lit(Vec3::ZERO, false), 0.0, "under the box");
    assert_eq!(lit(Vec3::new(2.0, 0.0, 0.0), false), 1.0, "beside it");
    assert_eq!(lit(Vec3::new(0.0, 1.0, 0.0), false), 1.0, "its own top");
    // Past a cascade's far end the depth counts as its far end.
    assert_eq!(lit(Vec3::new(0.0, -200.0, 0.0), false), 0.0);
    assert_eq!(lit(Vec3::new(2.0, -200.0, 0.0), false), 1.0);
    assert_eq!(
        lit(Vec3::new(900.0, 0.0, 0.0), false),
        1.0,
        "outside every cascade"
    );
    let none = GpuSunShadow::default();
    assert_eq!(
        sun_shadow(&none, Vec3::ZERO, Vec3::Y, 1.0, false, &sample),
        1.0
    );
}

#[test]
fn cascades_reach_past_their_slice_to_the_casters_under_them() {
    // Looking down under a sun overhead, every cascade covers the view's center. The near ones'
    // slices end high above the box standing on the ground, but they reach down to draw it, so
    // the ground past their far ends lies behind everything they hold.
    let above = Transform::from_xyz(0.0, 30.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z);
    let (center, radius) = (Vec3::new(0.0, 4.5, 0.0), 0.9);
    let open = fit_cascades(&viewer(above), Vec3::Y, DISTANCE, false, &[]);
    assert!(!open[0].touches(center, radius));
    let poses = fit_cascades(
        &viewer(above),
        Vec3::Y,
        DISTANCE,
        false,
        &[(center, radius)],
    );
    assert!(poses.iter().all(|pose| pose.touches(center, radius)));
    // Casters past the last slice don't stretch them.
    let deep = (Vec3::new(0.0, -500.0, 0.0), 1.0);
    let poses = fit_cascades(&viewer(above), Vec3::Y, DISTANCE, false, &[deep]);
    assert!(poses.iter().all(|pose| !pose.touches(deep.0, deep.1)));

    let (shadow, sample) = box_over(above, 5.0);
    let ground = Vec3::ZERO;
    let first = cascade_point(&shadow.cascades[0], ground, Vec3::Y);
    assert!(first.rim < 0.5 && first.depth == 1.0, "{first:?}");
    assert_eq!(
        sun_shadow(&shadow, ground, Vec3::Y, 1.0, false, &sample),
        0.0
    );
    let beside = Vec3::new(2.0, 0.0, 0.0);
    assert_eq!(
        sun_shadow(&shadow, beside, Vec3::Y, 1.0, false, &sample),
        1.0
    );
}

#[test]
fn static_receivers_of_moving_casters_keep_the_bakes_terminator() {
    let (mut shadow, sample) = box_scene();
    let point = Vec3::new(2.0, 0.0, 0.0);
    let turned = |shadow: &GpuSunShadow, cosine, static_receiver| {
        sun_shadow(shadow, point, Vec3::Y, cosine, static_receiver, &sample)
    };
    assert!(
        turned(&shadow, -0.05, true) < 0.1,
        "the map holds this receiver"
    );
    assert_eq!(turned(&shadow, -0.2, true), 0.0);
    shadow.flags = GpuSunShadow::FLAG_MOVING_ONLY;
    assert_eq!(turned(&shadow, -0.05, true), 1.0, "the bake shades it");
    assert!(turned(&shadow, -0.05, false) < 0.1, "a moving receiver");
}
