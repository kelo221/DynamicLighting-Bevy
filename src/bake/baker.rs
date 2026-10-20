//! The bake entry point: Henry's `StartRaytracing` as one pure function.

use rayon::prelude::*;

use crate::light::LightShape;
use crate::shared::DynamicGeometryMode;

use super::distance_cubes::{DISTANCE_CUBE_RESOLUTION, bake_distance_cubes};
use super::format::{BakeStats, BakedLight, BakedScene, BakedSettings, SunMap};
use super::input::{BakeError, BakeInput, BakeLight, BakeMesh, BakeOptions};
use super::light_bvh::build_light_bvh;
use super::mesh_bake::{MeshOutput, Scene, bake_mesh};
use super::mesh_builder::PreparedMesh;
use super::mirror::MAX_TRIANGLE_LIGHTS;
use super::occluder::Occluders;
use super::progress::{BakeProgress, BakeStage};
use super::sky_probes::bake_sky_probes;
use super::sun_map::{bake_sun_map, occluder_vertices};

/// Bakes `input`. Runs on the current rayon thread pool and gives byte-identical results for
/// the same input and options on any number of threads.
///
/// Steps, in Henry's order: light BVH (fixing the baked light order; suns, then skies, follow
/// the BVH's lights), occluder BVH, photon cubes for bounce lights, then per mesh: light
/// association, shadow rays, bounce, optimization, seam padding, dilation and serialization;
/// finally the distance cubes of the BVH's lights, the suns' sun maps and the sky probes.
pub fn bake(
    input: &BakeInput,
    options: &BakeOptions,
    progress: &BakeProgress,
) -> Result<BakedScene, BakeError> {
    options.validate()?;
    progress.begin(BakeStage::Preparing, 0);

    let meshes = sorted_meshes(&input.meshes)?;
    let (local, mut distant): (Vec<&BakeLight>, Vec<&BakeLight>) = sorted_lights(&input.lights)?
        .into_iter()
        .partition(|light| !light.shape.is_distant());
    distant.sort_by_key(|light| light.shape == LightShape::Sky);

    let positions: Vec<_> = local.iter().map(|l| l.position).collect();
    let radii: Vec<_> = local.iter().map(|l| l.radius).collect();
    let light_bvh = build_light_bvh(&positions, &radii);
    let local_lights = local.len();
    let lights: Vec<&BakeLight> = light_bvh
        .order
        .iter()
        .map(|&i| local[i])
        .chain(distant)
        .collect();
    let bounce_in_scene = lights.iter().any(|light| light.bakes_bounce(options));

    let occluders = Occluders::build(&meshes);
    progress.check()?;

    let bounce_lights = lights.iter().filter(|l| l.bakes_bounce(options)).count();
    progress.begin(BakeStage::PhotonCubes, bounce_lights as u64);
    let bounce = super::bounce::bounce_sources(&lights, &occluders, options, progress);
    progress.check()?;

    let mut scene = Scene {
        options,
        lights,
        local_lights,
        light_bvh,
        occluders,
        bounce_in_scene,
        bounce,
        progress,
    };

    let prepared: Vec<PreparedMesh> = meshes
        .iter()
        .map(|mesh| PreparedMesh::new(mesh, options))
        .collect();
    for mesh in prepared.iter().filter(|m| m.non_finite_uvs > 0) {
        bevy::log::warn!(
            "dynlight bake: mesh {:#018x} has {} non-finite UV1 coordinates, treated as (0, 0)",
            mesh.mesh.id.0,
            mesh.non_finite_uvs
        );
    }

    progress.begin(BakeStage::Meshes, prepared.len() as u64);
    let outputs: Vec<MeshOutput> = prepared
        .par_iter()
        .map(|mesh| {
            let output = bake_mesh(&scene, mesh);
            progress.advance(1);
            output
        })
        .collect::<Result<_, _>>()?;
    scene.bounce.clear();
    for output in outputs.iter().filter(|output| output.crowded.triangles > 0) {
        bevy::log::warn!(
            "dynlight bake: mesh {:#018x} has {} triangles lit by more than {MAX_TRIANGLE_LIGHTS} \
             baked lights (up to {}); shaders read only the first {MAX_TRIANGLE_LIGHTS}, so the \
             others leave dark seams. Split large triangles or use fewer overlapping lights",
            output.mesh.id.0,
            output.crowded.triangles,
            output.crowded.most_lights
        );
    }

    let distant = &scene.lights[local_lights..];
    let suns: Vec<_> = distant
        .iter()
        .filter_map(|l| Some((l.shape.sun()?, l)))
        .collect();
    let skies: Vec<_> = distant
        .iter()
        .filter(|l| l.shape == LightShape::Sky)
        .collect();
    let maps = options.dynamic_geometry != DynamicGeometryMode::LightingOnly;
    let map_count = if maps { suns.len() } else { 0 };
    let probes = maps && !skies.is_empty();
    progress.begin(
        BakeStage::DistanceCubes,
        local_lights as u64 * 6 + map_count as u64 + u64::from(probes),
    );
    let distance_cubes = bake_distance_cubes(
        &scene.lights[..local_lights],
        &scene.occluders,
        options.dynamic_geometry,
        progress,
    )?;
    let sun_maps: Vec<SunMap> = suns[..map_count]
        .iter()
        .map(|(sun, light)| {
            bake_sun_map(
                sun,
                &scene.occluders,
                occluder_vertices(&scene.occluders),
                options.sun_map_resolution,
                light.alpha_tested(),
                progress,
            )
        })
        .collect();
    progress.check()?;
    let sky_probes = if probes {
        bake_sky_probes(
            &scene.occluders,
            options.sky_probe_spacing,
            options.sky_samples,
            skies.iter().any(|light| light.alpha_tested()),
            options.seed,
            progress,
        )?
    } else {
        None
    };

    let mut stats = BakeStats {
        meshes: outputs.len() as u32,
        triangles: prepared.iter().map(|m| m.triangle_count() as u64).sum(),
        ..Default::default()
    };
    for output in &outputs {
        stats.texels += output.texels;
        stats.rays += output.rays;
        stats.bounce_rays += output.bounce_rays;
        stats.lights_removed += output.lights_removed;
        stats.triangle_words += output.mesh.triangles.len() as u64;
    }

    let baked = BakedScene {
        settings: BakedSettings {
            bounce: bounce_in_scene,
            colored_bounce: bounce_in_scene && options.colored_bounce,
            dynamic_geometry: options.dynamic_geometry,
            pixel_density: options.pixel_density,
            max_lightmap_size: options.max_lightmap_size,
            distance_cube_resolution: DISTANCE_CUBE_RESOLUTION,
        },
        lights: scene
            .lights
            .iter()
            .map(|light| BakedLight {
                id: light.id,
                origin: light.position,
                radius: light.radius,
                shape: light.shape,
                shadow_bits_per_texel: light.shadow_bits_per_texel(),
                bounce_compression: light.bakes_bounce(options).then(|| {
                    light
                        .bounce
                        .compression
                        .unwrap_or(options.bounce_compression)
                }),
            })
            .collect(),
        light_bvh: scene.light_bvh.nodes.clone(),
        meshes: outputs.into_iter().map(|output| output.mesh).collect(),
        distance_cubes,
        sun_maps,
        sky_probes,
        stats,
    };
    progress.begin(BakeStage::Done, 0);
    Ok(baked)
}

fn sorted_meshes(meshes: &[BakeMesh]) -> Result<Vec<&BakeMesh>, BakeError> {
    let mut sorted: Vec<&BakeMesh> = meshes.iter().collect();
    sorted.sort_by_key(|mesh| mesh.id);
    if let Some(pair) = sorted.windows(2).find(|pair| pair[0].id == pair[1].id) {
        return Err(BakeError::DuplicateId {
            kind: "meshes",
            id: pair[0].id,
        });
    }
    for mesh in &sorted {
        mesh.validate()?;
    }
    Ok(sorted)
}

fn sorted_lights(lights: &[BakeLight]) -> Result<Vec<&BakeLight>, BakeError> {
    let mut sorted: Vec<&BakeLight> = lights.iter().collect();
    sorted.sort_by_key(|light| light.id);
    if let Some(pair) = sorted.windows(2).find(|pair| pair[0].id == pair[1].id) {
        return Err(BakeError::DuplicateId {
            kind: "lights",
            id: pair[0].id,
        });
    }
    for light in &sorted {
        light.validate()?;
    }
    Ok(sorted)
}
