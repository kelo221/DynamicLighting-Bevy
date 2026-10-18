//! Area lights bake as one light with soft shadows: per texel, the share of the shape that
//! reaches it, in 4 bits. Point-like lights keep Henry's single shadow bit.

use std::time::Duration;

use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use dynlight::bake::mirror::TriangleView;
use dynlight::bake::{BakeFinished, BakePreset, BakeRequest, BakedScene, DynLightBake};
use dynlight::light::LightShape;
use dynlight::prelude::*;

const LAMP: Vec3 = Vec3::new(0.3, 3.0, 0.2);
/// The floor is 6 × 6 m, centered on the origin.
const FLOOR: f32 = 6.0;
/// The box spans 1 m around (0, 1, 0).
const BOX_MIN: Vec3 = Vec3::new(-0.5, 0.5, -0.5);
const BOX_MAX: Vec3 = Vec3::new(0.5, 1.5, 0.5);

/// Bakes a floor, a floating box and `light` 3 m above the floor.
fn bake_with(light: DynamicLight, rotation: Quat, preset: BakePreset) -> BakedScene {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default(), TransformPlugin))
        .init_asset::<Mesh>()
        .add_plugins(DynamicLightingPlugin)
        .insert_resource(BakeSettings {
            pixel_density: 8.0,
            ..default()
        });
    app.finish();
    app.cleanup();

    let mut floor = Plane3d::default().mesh().size(FLOOR, FLOOR).build();
    let uv0 = floor.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().clone();
    floor.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv0);
    let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
    let floor = meshes.add(floor);
    let cube = meshes.add(Cuboid::from_corners(BOX_MIN, BOX_MAX).mesh().build());
    let center = (BOX_MIN + BOX_MAX) / 2.0;
    let world = app.world_mut();
    world.spawn((Name::new("Floor"), Mesh3d(floor), BakeStatic));
    world.spawn((
        Name::new("Box"),
        Mesh3d(cube),
        BakeStatic,
        Transform::from_translation(center),
    ));
    world.spawn((
        Name::new("Lamp"),
        light,
        Transform::from_translation(LAMP).with_rotation(rotation),
    ));
    app.update();

    app.world_mut().write_message(BakeRequest {
        output: None,
        preset,
    });
    for _ in 0..6000 {
        app.update();
        let finished: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<BakeFinished>>()
            .drain()
            .collect();
        if let Some(finished) = finished.into_iter().next() {
            let report = finished.result.expect("bake succeeds");
            let assets = app.world().resource::<Assets<DynLightBake>>();
            return assets
                .get(&report.handle)
                .expect("baked asset")
                .scene
                .clone();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the bake didn't finish");
}

/// The floor's shadow map: per texel (row-major), how much of the scene's only light reaches
/// it, read like the shader does, and the texel's world position.
fn floor_shadows(scene: &BakedScene) -> (u32, Vec<(f32, Vec3)>) {
    let floor = scene
        .meshes
        .iter()
        .find(|mesh| mesh.id == BakeId::from_path(["Floor"]))
        .expect("derived floor id");
    let bits = scene.lights[0].shadow_bits_per_texel;
    let size = floor.resolution;
    let mut texels = Vec::new();
    for y in 0..size {
        for x in 0..size {
            // The plane's two triangles split along `u + v = 1`.
            let triangle = if x + y + 1 > size { 0 } else { 1 };
            let view = TriangleView::load(&floor.triangles, triangle, scene.settings.bounce);
            let value = (0..view.light_count)
                .map(|k| view.light(k))
                .find_map(|entry| view.shadow_value(&entry, x, y, bits))
                .unwrap_or(0.0);
            let uv = (Vec2::new(x as f32, y as f32) + 0.5) / size as f32;
            let world = Vec3::new(uv.x - 0.5, 0.0, uv.y - 0.5) * FLOOR;
            texels.push((value, world));
        }
    }
    (size, texels)
}

/// Whether the segment from `from` to `to` passes through the box.
fn hits_box(from: Vec3, to: Vec3) -> bool {
    let direction = to - from;
    let (mut near, mut far) = (0.0f32, 1.0f32);
    for axis in 0..3 {
        let (start, step) = (from[axis], direction[axis]);
        if step.abs() < 1e-9 {
            if start < BOX_MIN[axis] || start > BOX_MAX[axis] {
                return false;
            }
            continue;
        }
        let a = (BOX_MIN[axis] - start) / step;
        let b = (BOX_MAX[axis] - start) / step;
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    near <= far
}

/// The share of a downward panel's light reaching floor point `world`, from a dense grid of
/// points over the panel weighted by both cosines over distance².
fn brute_force_panel(world: Vec3, half: Vec2) -> f32 {
    const STEPS: usize = 48;
    let (mut lit, mut total) = (0.0, 0.0);
    for j in 0..STEPS {
        for i in 0..STEPS {
            let uv = (Vec2::new(i as f32, j as f32) + 0.5) / STEPS as f32 * 2.0 - 1.0;
            let point = LAMP + Vec3::new(uv.x * half.x, 0.0, uv.y * half.y);
            let to_point = point - world;
            let weight = (to_point.y / to_point.length()).powi(2) / to_point.length_squared();
            total += weight;
            if !hits_box(world, point) {
                lit += weight;
            }
        }
    }
    lit / total
}

#[test]
fn panels_bake_their_shape_with_soft_shadows() {
    let down = Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2);
    let panel = DynamicLight {
        radius: 6.0,
        ..DynamicLight::rectangle(1.5, 1.5)
    };
    let scene = bake_with(panel, down, BakePreset::Full);
    assert_eq!(scene.lights.len(), 1, "one light for the whole panel");
    let light = &scene.lights[0];
    assert_eq!((light.origin, light.radius), (LAMP, 6.0));
    assert_eq!(light.shadow_bits_per_texel, 4);
    let shape = light.shape.area().expect("the panel's shape");
    assert_eq!(shape.kind, LightType::Rectangle);
    assert!(shape.rotation.abs_diff_eq(down, 1e-6));
    assert_eq!(shape.size, Vec2::splat(1.5));

    let (_, texels) = floor_shadows(&scene);
    let errors: Vec<f32> = texels
        .iter()
        .map(|&(value, world)| (value - brute_force_panel(world, Vec2::splat(0.75))).abs())
        .collect();
    let mean = errors.iter().sum::<f32>() / errors.len() as f32;
    let worst = errors.iter().fold(0.0f32, |a, &b| a.max(b));
    assert!(mean < 0.02 && worst < 0.2, "mean {mean}, worst {worst}");
    // A real penumbra: the umbra, the lit floor and many levels between them.
    let levels: std::collections::BTreeSet<u32> = texels
        .iter()
        .map(|&(v, _)| (v * 15.0).round() as u32)
        .collect();
    assert!(
        levels.contains(&0) && levels.contains(&15) && levels.len() >= 12,
        "{levels:?}"
    );
}

/// Texels strictly between shadow and light along the most shadowed row and column.
fn penumbra_across(size: u32, texels: &[(f32, Vec3)]) -> (usize, usize) {
    let value = |x: u32, y: u32| texels[(x + y * size) as usize].0;
    let darkest = |line: &dyn Fn(u32) -> Vec<f32>| {
        (0..size)
            .map(line)
            .min_by(|a, b| a.iter().sum::<f32>().total_cmp(&b.iter().sum::<f32>()))
            .unwrap()
    };
    let soft = |line: Vec<f32>| line.iter().filter(|&&v| v > 0.05 && v < 0.95).count();
    let row = darkest(&|y| (0..size).map(|x| value(x, y)).collect());
    let column = darkest(&|x| (0..size).map(|y| value(x, y)).collect());
    (soft(row), soft(column))
}

#[test]
fn tubes_soften_shadows_only_along_their_length() {
    let tube = DynamicLight {
        radius: 6.0,
        ..DynamicLight::tube(1.0)
    };
    // Along world X (the floor's U), then turned to world Z (its V).
    let along_x = bake_with(tube.clone(), Quat::IDENTITY, BakePreset::Preview);
    let along_z = bake_with(
        tube,
        Quat::from_rotation_y(core::f32::consts::FRAC_PI_2),
        BakePreset::Preview,
    );
    assert_eq!(
        along_x.lights[0].shape.area().map(|a| a.kind),
        Some(LightType::Tube)
    );
    let (size, texels) = floor_shadows(&along_x);
    let (row, column) = penumbra_across(size, &texels);
    assert!(row >= 6 && column <= 1, "row {row}, column {column}");
    let (size, texels) = floor_shadows(&along_z);
    let (row, column) = penumbra_across(size, &texels);
    assert!(column >= 6 && row <= 1, "row {row}, column {column}");
}

#[test]
fn point_lights_keep_one_shadow_bit() {
    let point = DynamicLight {
        radius: 6.0,
        ..DynamicLight::point()
    };
    let scene = bake_with(point, Quat::IDENTITY, BakePreset::Preview);
    let light = &scene.lights[0];
    assert_eq!(
        (light.shadow_bits_per_texel, light.shape),
        (1, LightShape::Point)
    );
    assert!(scene.stats.rays > 0 && scene.stats.texels > 0);
    let (_, texels) = floor_shadows(&scene);
    assert!(texels.iter().all(|&(v, _)| v == 0.0 || v == 1.0));
    assert!(
        texels.iter().any(|&(v, _)| v == 0.0),
        "the box casts a shadow"
    );
}
