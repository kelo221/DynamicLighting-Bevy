//! Meshes that cast no shadows: the bake traces through them everywhere, as if they weren't
//! there, but still bakes their own light.

use bevy::math::{Quat, Vec2, Vec3};

use super::{area_light, cuboid, floor, point_light, sky_light, sun_light};
use crate::bake::input::{BakeInput, BakeOptions};
use crate::bake::mirror::TriangleView;
use crate::bake::progress::BakeProgress;
use crate::bake::{BakedScene, bake};
use crate::light::Illumination;
use crate::shared::{BakeId, DynamicGeometryMode};

fn options() -> BakeOptions {
    BakeOptions {
        pixel_density: 8.0,
        dynamic_geometry: DynamicGeometryMode::Angular,
        photon_cube_resolution: 32,
        sun_map_resolution: 64,
        sky_samples: 64,
        ..Default::default()
    }
}

/// A floor and a box above it, lit by a bouncing point light, an area light, a sun and a sky.
fn scene(occludes: Option<bool>) -> BakeInput {
    let mut point = point_light(10, Vec3::new(0.0, 3.0, 0.0), 8.0);
    point.illumination = Illumination::SingleBounce;
    point.bounce.samples = 8;
    let area = area_light(
        11,
        Vec3::new(1.0, 2.5, 0.5),
        8.0,
        Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2),
        Vec2::new(0.5, 0.5),
    );
    let mut meshes = vec![floor(1, Vec3::ZERO, 2.0)];
    if let Some(occludes) = occludes {
        let mut block = cuboid(2, Vec3::new(-0.5, 0.5, -0.5), Vec3::new(0.5, 1.0, 0.5));
        block.occludes = occludes;
        meshes.push(block);
    }
    BakeInput {
        meshes,
        lights: vec![
            point,
            area,
            sun_light(12, Vec3::new(0.3, 1.0, 0.2), 1.0),
            sky_light(13),
        ],
    }
}

fn bake_scene(occludes: Option<bool>) -> BakedScene {
    bake(&scene(occludes), &options(), &BakeProgress::new()).unwrap()
}

fn mesh(baked: &BakedScene, id: u64) -> &crate::bake::BakedMesh {
    baked.meshes.iter().find(|m| m.id == BakeId(id)).unwrap()
}

#[test]
fn meshes_that_cast_no_shadows_are_traced_through() {
    let open = bake_scene(None);
    let silent = bake_scene(Some(false));
    let solid = bake_scene(Some(true));
    assert_eq!(
        mesh(&silent, 1),
        mesh(&open, 1),
        "the floor, bounce included"
    );
    assert_ne!(mesh(&solid, 1).triangles, mesh(&open, 1).triangles);
    assert_eq!(silent.distance_cubes, open.distance_cubes);
    assert_ne!(solid.distance_cubes, open.distance_cubes);
    assert_eq!(silent.sun_maps, open.sun_maps);
    assert_ne!(solid.sun_maps, open.sun_maps);
    assert!(open.sky_probes.is_some());
    assert_eq!(silent.sky_probes, open.sky_probes);
    assert!(!mesh(&silent, 2).occludes && mesh(&solid, 2).occludes);
}

#[test]
fn meshes_that_cast_no_shadows_still_receive_light() {
    let silent = bake_scene(Some(false));
    let block = mesh(&silent, 2);
    // The top face (triangles 4 and 5 of the cuboid) faces the point light above it.
    let view = TriangleView::load(&block.triangles, 4, silent.settings.bounce);
    assert!(view.light_count > 0);
}

/// A decal: a small quad 1 mm above a floor, lit from above. Shadow rays leaving the decal must
/// not hit the floor right behind it.
#[test]
fn decals_just_above_a_surface_are_lit() {
    for gap in [0.01, 0.001, 0.0002] {
        let input = BakeInput {
            meshes: vec![floor(1, Vec3::ZERO, 2.0), floor(2, Vec3::Y * gap, 0.25)],
            lights: vec![point_light(10, Vec3::new(0.7, 2.0, 0.3), 8.0)],
        };
        let options = BakeOptions {
            pixel_density: 32.0,
            dynamic_geometry: DynamicGeometryMode::LightingOnly,
            ..Default::default()
        };
        let baked = bake(&input, &options, &BakeProgress::new()).unwrap();
        let decal = mesh(&baked, 2);
        let (mut lit, mut dark) = (0, 0);
        for triangle in 0..2 {
            let view = TriangleView::load(&decal.triangles, triangle, false);
            let entry = view.light(0);
            for y in 0..decal.resolution {
                for x in 0..decal.resolution {
                    match view.shadow_bit(&entry, x, y) {
                        Some(true) => lit += 1,
                        Some(false) => dark += 1,
                        None => {}
                    }
                }
            }
        }
        assert!(lit > 0 && dark == 0, "gap {gap}: {lit} lit, {dark} dark");
    }
}
