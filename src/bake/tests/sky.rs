//! Skies: texels see the share of the sky the analytic view factors leave, enclosed triangles
//! drop the sky, and the sky probes see the scene like texels facing their six axes.

use bevy::math::{UVec3, Vec2, Vec3};

use super::{cuboid, floor, sky_light};
use crate::bake::input::{BakeInput, BakeMesh, BakeOptions};
use crate::bake::mirror::TriangleView;
use crate::bake::progress::BakeProgress;
use crate::bake::texels::triangle_texels;
use crate::bake::{BakedScene, bake};
use crate::shared::DynamicGeometryMode;

fn options(sky_samples: u32) -> BakeOptions {
    BakeOptions {
        pixel_density: 8.0,
        dynamic_geometry: DynamicGeometryMode::LightingOnly,
        sky_samples,
        ..Default::default()
    }
}

fn bake_sky(meshes: Vec<BakeMesh>, options: &BakeOptions) -> BakedScene {
    let input = BakeInput {
        meshes,
        lights: vec![sky_light(10)],
    };
    bake(&input, options, &BakeProgress::new()).unwrap()
}

/// Every texel of `floor_mesh` (baked as mesh `id`) with its sky visibility, `None` where the
/// triangle has no sky entry.
fn floor_texels(baked: &BakedScene, floor_mesh: &BakeMesh) -> Vec<(Vec3, Option<f32>)> {
    let mesh = baked.meshes.iter().find(|m| m.id == floor_mesh.id).unwrap();
    let bits = baked.lights[0].shadow_bits_per_texel;
    let mut texels = Vec::new();
    for triangle in 0..2 {
        let corners = &floor_mesh.indices[triangle * 3..][..3];
        let vertices = [0, 1, 2].map(|k| floor_mesh.positions[corners[k] as usize]);
        let uvs: [Vec2; 3] = [0, 1, 2].map(|k| floor_mesh.uv1[corners[k] as usize]);
        let view = TriangleView::load(&mesh.triangles, triangle as u32, baked.settings.bounce);
        let entry = (0..view.light_count)
            .map(|k| view.light(k))
            .find(|e| e.light_index == 0);
        for t in triangle_texels(vertices, [Vec3::Y; 3], uvs, Vec3::Y, mesh.resolution) {
            let value = entry.and_then(|e| view.shadow_value(&e, t.x, t.y, bits));
            texels.push((t.position, value));
        }
    }
    texels
}

#[test]
fn roofed_floors_see_almost_no_sky_and_open_floors_all_of_it() {
    let roofed = floor(1, Vec3::ZERO, 2.0);
    // Above everything else, so no ray from it hits anything.
    let open = floor(2, Vec3::new(0.0, 30.0, 0.0), 1.0);
    let enclosed = floor(3, Vec3::new(0.0, 10.0, 0.0), 0.25);
    let meshes = vec![
        roofed.clone(),
        open.clone(),
        enclosed.clone(),
        cuboid(4, Vec3::new(-1.5, 0.1, -1.5), Vec3::new(1.5, 0.2, 1.5)),
        cuboid(5, Vec3::new(-1.0, 9.0, -1.0), Vec3::new(1.0, 11.0, 1.0)),
    ];
    let baked = bake_sky(meshes, &options(64));
    assert_eq!(baked.lights[0].shadow_bits_per_texel, 8);
    // At least 1 m in from the roof's edges, 0.1 m up, only rays within 5.7° of the horizon get
    // out: sin² 5.7° = 1%, up to two of the 64 rays.
    let under: Vec<f32> = floor_texels(&baked, &roofed)
        .into_iter()
        .filter(|(p, _)| p.x.abs() < 0.5 && p.z.abs() < 0.5)
        .map(|(_, value)| value.expect("some sky reaches under the roof"))
        .collect();
    assert!(!under.is_empty());
    assert!(under.iter().all(|&value| value < 0.035), "{under:?}");
    let mean = under.iter().sum::<f32>() / under.len() as f32;
    assert!(mean < 0.015, "{mean}");
    let open = floor_texels(&baked, &open);
    assert!(
        open.iter().all(|&(_, value)| value == Some(1.0)),
        "{open:?}"
    );
    let enclosed = floor_texels(&baked, &enclosed);
    assert!(enclosed.iter().all(|(_, value)| value.is_none()), "dropped");
    assert!(baked.stats.lights_removed >= 2);
}

#[test]
fn a_long_wall_hides_the_analytic_share_of_the_sky() {
    // Beside an endless wall of height h, a floor point d away sees ½ (1 + d / √(d² + h²)) of a
    // uniform sky (the two-dimensional view factor of the wall is ½ (1 − d / √(d² + h²))).
    let height = 1.0;
    let ground = floor(1, Vec3::new(-2.0, 0.0, 0.0), 2.0);
    let wall = cuboid(2, Vec3::new(0.0, 0.0, -20.0), Vec3::new(0.1, height, 20.0));
    let baked = bake_sky(vec![ground.clone(), wall], &options(256));
    let mut columns = std::collections::BTreeMap::<i32, (f32, f32, u32)>::new();
    for (position, value) in floor_texels(&baked, &ground) {
        let distance = -position.x;
        if position.z.abs() > 1.5 || distance < 0.2 {
            continue;
        }
        let expected = 0.5 * (1.0 + distance / (distance * distance + height * height).sqrt());
        let value = value.expect("the wall leaves some sky");
        assert!(
            (value - expected).abs() < 0.06,
            "{distance}: {value} {expected}"
        );
        let column = columns.entry((distance * 1000.0) as i32).or_default();
        *column = (column.0 + value, column.1 + expected, column.2 + 1);
    }
    assert!(columns.len() > 20);
    for (distance, (value, expected, count)) in columns {
        let (value, expected) = (value / count as f32, expected / count as f32);
        assert!(
            (value - expected).abs() < 0.02,
            "{distance}: {value} {expected}"
        );
    }
}

#[test]
fn sky_probes_see_the_roof_and_fill_from_above_under_the_floor() {
    let meshes = vec![
        floor(1, Vec3::ZERO, 3.0),
        cuboid(2, Vec3::new(-1.0, 1.4, -1.0), Vec3::new(1.0, 1.6, 1.0)),
    ];
    let options = BakeOptions {
        dynamic_geometry: DynamicGeometryMode::Angular,
        sky_probe_spacing: 1.0,
        ..options(1024)
    };
    let baked = bake_sky(meshes.clone(), &options);
    let probes = baked.sky_probes.as_ref().expect("a sky bakes probes");
    assert_eq!(probes.counts, UVec3::new(8, 4, 8));
    assert_eq!(probes.origin, Vec3::new(-3.5, -0.7, -3.5));
    for (x, z) in (0..8).flat_map(|x| (0..8).map(move |z| (x, z))) {
        assert_eq!(
            probes.openness(UVec3::new(x, 3, z))[2],
            1.0,
            "above everything"
        );
    }
    // 0.3 m over the floor and 1.1 m under the roof, which reaches 0.5 m and 1.5 m past the
    // probe on both sides: the view factor of the roof's four parts is 0.411.
    let [_, _, up, down, _, _] = probes.openness(UVec3::new(3, 1, 3));
    assert!((up - 0.589).abs() < 0.01, "{up}");
    assert!(down < 0.05, "{down}");
    // The probe under the floor mostly sees its back, so it takes the probe's above it.
    let above = probes.openness(UVec3::new(3, 1, 3));
    assert_eq!(probes.openness(UVec3::new(3, 0, 3)), above);
    let lighting_only = bake_sky(meshes, &self::options(16));
    assert!(lighting_only.sky_probes.is_none());
    let empty = bake_sky(Vec::new(), &options);
    assert!(empty.sky_probes.is_none(), "nothing to shade");
}
