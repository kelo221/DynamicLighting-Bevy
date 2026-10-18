//! GPU readback check of a baked sun: a slanted sun with a 3° disk over slabs and columns, baked
//! in memory at startup. The left floor is baked and reads the soft shadow levels; the right
//! floor isn't, so it reads the sun's sun map. Sampled pixels are compared with the CPU mirrors
//! (`baked_lighting::mirror`, `dynamic_geometry::sun_mirror`) evaluated on the same bake.
//!
//! `cargo run --example gpu_check_sun -- [--quality integrated|low|medium|high]` prints the
//! largest difference and exits with an error code when a sample is off. A sample passes when it
//! lies within 1% (plus 0.01) of the CPU values over a quarter pixel around its center.

use bevy::camera::{Hdr, ImageRenderTarget, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use dynlight::bake::mirror::TriangleView;
use dynlight::bake::{BakeFinished, BakePreset, BakeRequest, BakedScene, DynLightBake};
use dynlight::baked_lighting::ActiveBake;
use dynlight::baked_lighting::mirror::{ShadowTexels, log_bits};
use dynlight::dynamic_geometry::sun_mirror::sun_map_shadow;
use dynlight::light::{PackedLights, mirror};
use dynlight::prelude::*;
use dynlight::shared::RuntimeQuality;

/// Image side in pixels: 256 `Rgba16Float` texels are 2048 bytes, so rows have no padding.
const SIZE: u32 = 256;
/// Side of the view, in meters; each floor is half of it.
const EXTENT: f32 = 10.0;
const STEP: u32 = 4;
const RELATIVE_TOLERANCE: f32 = 0.01;
const ABSOLUTE_TOLERANCE: f32 = 0.01;
/// Real time after the bake is applied before readbacks are checked, and until a matching one
/// may arrive.
const WARM_UP_SECONDS: f32 = 2.0;
const DEADLINE_SECONDS: f32 = 60.0;
/// Samples on each floor that must be at least half in shadow, so the shadows are tested.
const MIN_SHADOWED: usize = 20;
/// Boxes: center and half size, a slab and a column over each floor. Pixels over them (seen
/// from above) aren't checked.
const BOXES: [(Vec3, Vec3); 4] = [
    (Vec3::new(-2.5, 1.0, -1.5), Vec3::new(0.6, 0.1, 0.6)),
    (Vec3::new(-3.0, 0.75, 2.0), Vec3::new(0.2, 0.75, 0.2)),
    (Vec3::new(2.0, 1.0, -1.0), Vec3::new(0.6, 0.1, 0.6)),
    (Vec3::new(2.5, 0.75, 2.5), Vec3::new(0.2, 0.75, 0.2)),
];

fn main() -> AppExit {
    let quality = match std::env::args()
        .skip_while(|a| a != "--quality")
        .nth(1)
        .as_deref()
    {
        Some("integrated") => RuntimeQuality::IntegratedGraphics,
        Some("low") => RuntimeQuality::Low,
        Some("medium") => RuntimeQuality::Medium,
        _ => RuntimeQuality::High,
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "DynLight sun GPU check".into(),
                resolution: (512, 512).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(DynamicLightingPlugin)
        .insert_resource(DynamicLightingSettings {
            ambient: Color::NONE,
            quality,
            ..default()
        })
        .insert_resource(BakeSettings {
            pixel_density: 32.0,
            sun_map_resolution: 512,
            ..default()
        })
        .init_resource::<AppliedAt>()
        .add_systems(Startup, setup)
        .add_systems(Update, (request_bake, apply_bake))
        .run()
}

/// When the bake was applied, in real seconds.
#[derive(Resource, Default)]
struct AppliedAt(Option<f32>);

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    let mut target = Image::new_target_texture(SIZE, SIZE, TextureFormat::Rgba16Float, None);
    target.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = images.add(target);

    let white = materials.add(Color::WHITE);
    let mut baked = Plane3d::default().mesh().size(EXTENT / 2.0, EXTENT).build();
    let uv0 = baked.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().clone();
    baked.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv0);
    commands.spawn((
        Name::new("Baked floor"),
        Mesh3d(meshes.add(baked)),
        MeshMaterial3d(white.clone()),
        Transform::from_xyz(-EXTENT / 4.0, 0.0, 0.0),
        BakeStatic,
    ));
    commands.spawn((
        Name::new("Dynamic floor"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(EXTENT / 2.0, EXTENT))),
        MeshMaterial3d(white.clone()),
        Transform::from_xyz(EXTENT / 4.0, 0.0, 0.0),
    ));
    for (i, (center, half)) in BOXES.into_iter().enumerate() {
        commands.spawn((
            Name::new(format!("Box {i}")),
            Mesh3d(meshes.add(Cuboid::from_size(half * 2.0))),
            MeshMaterial3d(white.clone()),
            Transform::from_translation(center),
            BakeStatic,
        ));
    }
    let (sun, transform) = sun_light(Dir3::new(Vec3::new(-0.5, -1.0, -0.3)).unwrap());
    commands.spawn((
        Name::new("Sun"),
        DynamicLight {
            color: Color::srgb(1.0, 0.95, 0.85),
            angular_diameter: 3.0,
            ..sun
        },
        transform,
    ));

    let view = (
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: EXTENT,
                height: EXTENT,
            },
            ..OrthographicProjection::default_3d()
        }),
        // Image right is +X and image down is +Z.
        Transform::from_xyz(0.0, 10.0, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
    );
    commands.spawn((
        Camera3d::default(),
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        RenderTarget::Image(ImageRenderTarget::from(target.clone())),
        view.clone(),
    ));
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 1,
            ..default()
        },
        view,
    ));
    commands.spawn(Readback::texture(target)).observe(check);
}

/// Bakes once the scene's transforms are up to date.
fn request_bake(mut frames: Local<u32>, mut requests: MessageWriter<BakeRequest>) {
    *frames += 1;
    if *frames == 3 {
        requests.write(BakeRequest {
            output: None,
            preset: BakePreset::Full,
        });
    }
}

fn apply_bake(
    mut finished: MessageReader<BakeFinished>,
    mut active: ResMut<ActiveBake>,
    mut applied: ResMut<AppliedAt>,
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
) {
    for bake in finished.read() {
        match &bake.result {
            Ok(report) => {
                active.0 = Some(report.handle.clone());
                applied.0 = Some(time.elapsed_secs());
            }
            Err(failure) => {
                error!("bake failed: {failure}");
                exit.write(AppExit::error());
            }
        }
    }
}

/// The world position seen at pixel coordinates `(x, y)` (pixel centers at `.5`).
fn pixel_world(x: f32, y: f32) -> Vec3 {
    let at = |pixel: f32| (pixel / SIZE as f32 - 0.5) * EXTENT;
    Vec3::new(at(x), 0.0, at(y))
}

/// The baked floor's shadow of light `slot` at `world`, as the triangle shader reads it; `None`
/// when the triangle doesn't list the light.
fn baked_shadow(scene: &BakedScene, slot: usize, world: Vec3, quality: u32) -> Option<f32> {
    let floor = scene
        .meshes
        .iter()
        .find(|mesh| mesh.id == BakeId::from_path(["Baked floor"]))?;
    let uv = Vec2::new(
        (world.x + EXTENT / 4.0) / (EXTENT / 2.0) + 0.5,
        world.z / EXTENT + 0.5,
    );
    // The plane's two triangles split along `u + v = 1`.
    let triangle = if uv.x + uv.y >= 1.0 { 0 } else { 1 };
    let view = TriangleView::load(&floor.triangles, triangle, scene.settings.bounce);
    let entry = (0..view.light_count)
        .map(|k| view.light(k))
        .find(|entry| entry.light_index as usize == slot)?;
    if entry.shadow_offset == 0 {
        return Some(1.0);
    }
    let texels = ShadowTexels {
        words: &floor.triangles[entry.shadow_offset as usize..],
        width: view.bounds[2],
        origin: UVec2::new(view.bounds[0], view.bounds[1]),
        log_bits: log_bits(scene.lights[slot].shadow_bits_per_texel),
    };
    Some(texels.shadow(uv * floor.resolution as f32, quality))
}

/// The floor color the shaders should produce at `world` with the bake applied, and the
/// darkest shadow of the suns.
fn expected(scene: &BakedScene, lights: &PackedLights, world: Vec3) -> (Vec3, f32) {
    let quality = lights.globals.quality;
    let baked = world.x < 0.0;
    let local = scene.local_light_count();
    let slots = lights
        .lights
        .iter()
        .take(lights.globals.baked_count as usize);
    slots
        .enumerate()
        .skip(local)
        .fold((Vec3::ZERO, 1.0f32), |(sum, darkest), (slot, light)| {
            let shadow = if baked {
                baked_shadow(scene, slot, world, quality).unwrap_or(0.0)
            } else {
                scene
                    .sun_maps
                    .get(slot - local)
                    .map_or(1.0, |map| sun_map_shadow(map, world))
            };
            let lit =
                mirror::radiance(light, world) * mirror::diffuse(light, world, Vec3::Y, baked);
            (
                sum + Vec3::from(light.color) * lit * shadow,
                darkest.min(shadow),
            )
        })
}

/// Pixels whose quarter-pixel neighbourhood stays on one floor and off the boxes.
fn checked(x: f32, y: f32) -> bool {
    let margin = 1.5 * EXTENT / SIZE as f32;
    let world = pixel_world(x, y);
    world.x.abs() > margin
        && BOXES.iter().all(|(center, half)| {
            (world.x - center.x).abs() > half.x + margin
                || (world.z - center.z).abs() > half.z + margin
        })
}

fn check(
    readback: On<ReadbackComplete>,
    time: Res<Time<Real>>,
    applied: Res<AppliedAt>,
    (active, bakes): (Res<ActiveBake>, Res<Assets<DynLightBake>>),
    lights: Res<PackedLights>,
    mut exit: MessageWriter<AppExit>,
    mut done: Local<bool>,
) {
    let since = applied.0.map_or(0.0, |at| time.elapsed_secs() - at);
    let Some(bake) = active.0.as_ref().and_then(|handle| bakes.get(handle)) else {
        return;
    };
    if *done || since < WARM_UP_SECONDS {
        return;
    }
    let texels: Vec<half::f16> = bytemuck::cast_slice::<u8, u16>(&readback.data)
        .iter()
        .map(|&bits| half::f16::from_bits(bits))
        .collect();
    let (mut worst, mut failures, mut samples) = ((0.0, Vec2::ZERO), 0, 0);
    // Samples at least half in shadow on the baked and the dynamic floor.
    let mut shadowed = [0usize; 2];
    for y in (STEP / 2..SIZE).step_by(STEP as usize) {
        for x in (STEP / 2..SIZE).step_by(STEP as usize) {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            if !checked(px, py) {
                continue;
            }
            let index = ((y * SIZE + x) * 4) as usize;
            let gpu = Vec3::from_array(core::array::from_fn(|c| texels[index + c].to_f32()));
            let around = [
                (0.0, 0.0),
                (-0.25, 0.0),
                (0.25, 0.0),
                (0.0, -0.25),
                (0.0, 0.25),
            ];
            let cpu: Vec<(Vec3, f32)> = around
                .iter()
                .map(|(dx, dy)| expected(&bake.scene, &lights, pixel_world(px + dx, py + dy)))
                .collect();
            let low = cpu.iter().fold(Vec3::MAX, |a, b| a.min(b.0));
            let high = cpu.iter().fold(Vec3::MIN, |a, b| a.max(b.0));
            let allowed = ABSOLUTE_TOLERANCE + RELATIVE_TOLERANCE * high.max_element();
            let error = (low - gpu).max(gpu - high).max(Vec3::ZERO).max_element();
            samples += 1;
            if cpu[0].1 < 0.5 {
                shadowed[usize::from(px > SIZE as f32 / 2.0)] += 1;
            }
            failures += usize::from(error > allowed);
            if error > worst.0 {
                worst = (error, Vec2::new(px, py));
            }
        }
    }
    let suns = bake.scene.sun_count();
    let passed = failures == 0 && suns == 1 && shadowed.iter().all(|&n| n >= MIN_SHADOWED);
    if !passed && since < DEADLINE_SECONDS {
        return;
    }
    *done = true;
    info!(
        "{samples} samples ({} and {} in shadow on the baked and dynamic floors), {suns} sun, \
         quality {}; largest difference {:.4} at pixel {}",
        shadowed[0], shadowed[1], lights.globals.quality, worst.0, worst.1
    );
    if passed {
        info!("GPU sun shadows match the CPU mirror");
        exit.write(AppExit::Success);
    } else {
        error!("{failures} of {samples} samples differ from the CPU mirror");
        exit.write(AppExit::error());
    }
}
