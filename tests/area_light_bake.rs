//! Area lights belong to Henry's family of light types: they are baked as one or more ordinary
//! point lights spread over their shape, each with 1-bit shadows from a fixed position.

use std::time::Duration;

use bevy::prelude::*;
use bevy::transform::TransformPlugin;
use dynlight::bake::mirror::TriangleView;
use dynlight::bake::{
    BakeFinished, BakePreset, BakeRequest, BakedScene, DynLightBake, decode, encode,
};
use dynlight::light::shadow_point_id;
use dynlight::prelude::*;

const LAMP: Vec3 = Vec3::new(0.3, 3.0, 0.2);

/// Bakes a floor, a floating box and `light` 3 m above the box, and returns the file bytes.
fn bake_with(light: DynamicLight, rotation: Quat) -> Vec<u8> {
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

    let mut floor = Plane3d::default().mesh().size(6.0, 6.0).build();
    let uv0 = floor.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().clone();
    floor.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv0);
    let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
    let floor = meshes.add(floor);
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0).mesh().build());
    let world = app.world_mut();
    world.spawn((Name::new("Floor"), Mesh3d(floor), BakeStatic));
    world.spawn((
        Name::new("Box"),
        Mesh3d(cube),
        BakeStatic,
        Transform::from_xyz(0.0, 1.0, 0.0),
    ));
    world.spawn((
        Name::new("Lamp"),
        light,
        Transform::from_translation(LAMP).with_rotation(rotation),
    ));
    app.update();

    app.world_mut().write_message(BakeRequest {
        output: None,
        preset: BakePreset::Preview,
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
            return encode(&assets.get(&report.handle).expect("baked asset").scene);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the bake didn't finish");
}

/// Whether each floor texel sees the scene's light `light`, read like the shader does. The
/// plane's two triangles split along `u + v = 1`.
fn floor_lit(scene: &BakedScene, light: usize) -> Vec<bool> {
    let floor = scene
        .meshes
        .iter()
        .find(|mesh| mesh.id == BakeId::from_path(["Floor"]))
        .expect("derived floor id");
    let size = floor.resolution;
    let mut lit = Vec::new();
    for y in 0..size {
        for x in 0..size {
            let triangle = if x + y + 1 > size { 0 } else { 1 };
            let view = TriangleView::load(&floor.triangles, triangle, scene.settings.bounce);
            let bit = (0..view.light_count)
                .map(|k| view.light(k))
                .find(|entry| entry.light_index as usize == light)
                .and_then(|entry| view.shadow_bit(&entry, x, y));
            lit.push(bit == Some(true));
        }
    }
    lit
}

fn single(light: DynamicLight) -> DynamicLight {
    DynamicLight {
        radius: 6.0,
        shadow_points: 1,
        ..light
    }
}

#[test]
fn single_point_area_lights_bake_like_point_lights() {
    let point = bake_with(single(DynamicLight::point()), Quat::IDENTITY);
    let panel = bake_with(
        single(DynamicLight::rectangle(1.5, 0.5)),
        Quat::from_rotation_x(-0.4),
    );
    let tube = bake_with(single(DynamicLight::tube(2.0)), Quat::from_rotation_y(0.9));
    let scene = decode(&point).unwrap();
    assert_eq!(scene.lights.len(), 1);
    assert!(scene.stats.rays > 0 && scene.stats.texels > 0);
    assert_eq!(panel, point, "a panel bakes the point light's shadow bits");
    assert_eq!(tube, point, "so does a tube");
}

#[test]
fn area_lights_bake_one_point_light_per_shadow_point() {
    let rotation = Quat::from_rotation_y(0.9);
    let tube = DynamicLight {
        radius: 6.0,
        ..DynamicLight::tube(2.0)
    };
    let scene = decode(&bake_with(tube.clone(), rotation)).unwrap();
    let point = decode(&bake_with(single(DynamicLight::point()), Quat::IDENTITY)).unwrap();
    assert_eq!(scene.lights.len(), 3);

    let lamp = BakeId::from_path(["Lamp"]);
    let mut lit = Vec::new();
    let mut slots = Vec::new();
    for shadow_point in tube.shadow_point_layout(LAMP, rotation) {
        let id = shadow_point_id(lamp, shadow_point.index, 3);
        let index = scene
            .lights
            .iter()
            .position(|light| light.id == id)
            .expect("every shadow point is baked");
        slots.push(index);
        let baked = &scene.lights[index];
        assert!(baked.origin.distance(shadow_point.position) < 1e-6);
        assert_eq!(baked.radius, shadow_point.radius);
        lit.push(floor_lit(&scene, index));
    }
    // The points fill consecutive slots in order, so the shaders can shade the tube once.
    assert_eq!(slots, [slots[0], slots[0] + 1, slots[0] + 2]);
    assert_eq!(
        lit[1],
        floor_lit(&point, 0),
        "the middle point sees what a point light at the position sees"
    );
    let differ = |a: &[bool], b: &[bool]| a.iter().zip(b).filter(|(a, b)| a != b).count();
    assert!(differ(&lit[0], &lit[1]) > 0 && differ(&lit[1], &lit[2]) > 0);
    // Texels only some points see form the penumbra; most texels agree.
    let penumbra = (0..lit[0].len())
        .filter(|&t| !(lit[0][t] == lit[1][t] && lit[1][t] == lit[2][t]))
        .count();
    assert!(penumbra > 0 && penumbra < lit[0].len() / 4, "{penumbra}");
}

#[test]
fn panel_shadows_soften_across_both_sides() {
    let down = Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2);
    let panel = DynamicLight {
        radius: 6.0,
        shadow_points: 4,
        ..DynamicLight::rectangle(1.5, 1.5)
    };
    let scene = decode(&bake_with(panel.clone(), down)).unwrap();
    assert_eq!(scene.lights.len(), 4);
    let lamp = BakeId::from_path(["Lamp"]);
    // Where on the floor each point's shadow of the box lies, in texels.
    let centers: Vec<Vec2> = panel
        .shadow_point_layout(LAMP, down)
        .map(|point| {
            let id = shadow_point_id(lamp, point.index, 4);
            let index = scene
                .lights
                .iter()
                .position(|light| light.id == id)
                .expect("every shadow point is baked");
            let lit = floor_lit(&scene, index);
            let size = (lit.len() as f32).sqrt() as usize;
            let (sum, count) = lit.iter().enumerate().filter(|(_, lit)| !**lit).fold(
                (Vec2::ZERO, 0),
                |(sum, count), (texel, _)| {
                    let at = Vec2::new((texel % size) as f32, (texel / size) as f32);
                    (sum + at, count + 1)
                },
            );
            sum / count as f32
        })
        .collect();
    let low = centers.iter().fold(Vec2::MAX, |low, &c| low.min(c));
    let high = centers.iter().fold(Vec2::MIN, |high, &c| high.max(c));
    let spread = high - low;
    // A row of points would shift the shadows along one floor axis only.
    assert!(spread.x > 3.0 && spread.y > 3.0, "{spread}");
}
