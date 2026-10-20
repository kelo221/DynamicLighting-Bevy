//! GPU readback check of colored bounce light: a red wall on a white floor, lit by a sun on one
//! side and a point light on the other, both baking colored bounce. The floor next to the wall
//! takes red light from both sides (the sun's gathered over the hemisphere, the point light's
//! from its photon cube). Sampled floor pixels are compared with the CPU mirrors
//! (`baked_lighting::mirror`, `light::mirror`) evaluated on the same bake.
//!
//! `cargo run --example gpu_check_bounce -- [--quality integrated|low|medium|high]` prints the
//! largest difference and exits with an error code when a sample is off. A sample passes when it
//! lies within 1% (plus 0.01) of the CPU values over a quarter pixel around its center.
//! Integrated-graphics quality skips bounce light, which the mirror follows.

use bevy::camera::{Hdr, ImageRenderTarget, RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::gpu_readback::{Readback, ReadbackComplete};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use dynlight::bake::mirror::TriangleView;
use dynlight::bake::{BakeFinished, BakePreset, BakeRequest, BakedScene, DynLightBake};
use dynlight::baked_lighting::ActiveBake;
use dynlight::baked_lighting::mirror::{BounceTexels, QUALITY_INTEGRATED, ShadowTexels, log_bits};
use dynlight::light::{PackedLights, mirror};
use dynlight::prelude::*;
use dynlight::shared::{ChannelFlags, GpuLight, RuntimeQuality};

/// Image side in pixels: 256 `Rgba16Float` texels are 2048 bytes, so rows have no padding.
const SIZE: u32 = 256;
/// Side of the floor and the view, in meters.
const EXTENT: f32 = 10.0;
const STEP: u32 = 4;
const RELATIVE_TOLERANCE: f32 = 0.01;
const ABSOLUTE_TOLERANCE: f32 = 0.01;
const WARM_UP_SECONDS: f32 = 2.0;
const DEADLINE_SECONDS: f32 = 60.0;
/// The wall: center and half size. Pixels over it aren't checked.
const WALL: (Vec3, Vec3) = (Vec3::new(1.0, 0.75, 0.0), Vec3::new(0.2, 0.75, 2.0));
/// Samples whose bounce light is clearly red, so colored bounce is tested.
const MIN_RED: usize = 40;

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
                title: "DynLight bounce GPU check".into(),
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
            pixel_density: 16.0,
            colored_bounce: true,
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

    let mut floor = Plane3d::default().mesh().size(EXTENT, EXTENT).build();
    let uv0 = floor.attribute(Mesh::ATTRIBUTE_UV_0).unwrap().clone();
    floor.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv0);
    commands.spawn((
        Name::new("Floor"),
        Mesh3d(meshes.add(floor)),
        MeshMaterial3d(materials.add(Color::WHITE)),
        BakeStatic,
    ));
    commands.spawn((
        Name::new("Wall"),
        Mesh3d(meshes.add(Cuboid::from_size(WALL.1 * 2.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.8, 0.1, 0.1))),
        Transform::from_translation(WALL.0),
        BakeStatic,
    ));
    let bounce = Illumination::SingleBounce;
    let (sun, transform) = sun_light(Dir3::new(Vec3::new(-0.6, -1.0, 0.1)).unwrap());
    commands.spawn((
        Name::new("Sun"),
        DynamicLight {
            illumination: bounce,
            ..sun
        },
        transform,
    ));
    commands.spawn((
        Name::new("Lamp"),
        DynamicLight {
            radius: 4.0,
            illumination: bounce,
            bounce: LightBounce {
                intensity: 2.0,
                ..default()
            },
            ..default()
        },
        Transform::from_xyz(-1.5, 1.2, 0.5),
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

/// Bakes once the scene's transforms and the derived albedos are up to date.
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

/// The floor's baked shadow and bounce light (before the light's color and strength) of light
/// `slot` at `world`, as the triangle shader reads them; `None` when the triangle doesn't list
/// the light.
fn floor_terms(
    scene: &BakedScene,
    light: &GpuLight,
    slot: usize,
    world: Vec3,
    quality: u32,
) -> Option<(f32, Vec3)> {
    let floor = scene
        .meshes
        .iter()
        .find(|mesh| mesh.id == BakeId::from_path(["Floor"]))?;
    let uv = Vec2::new(world.x / EXTENT + 0.5, world.z / EXTENT + 0.5);
    // The plane's two triangles split along `u + v = 1`.
    let triangle = if uv.x + uv.y >= 1.0 { 0 } else { 1 };
    let view = TriangleView::load(&floor.triangles, triangle, scene.settings.bounce);
    let entry = (0..view.light_count)
        .map(|k| view.light(k))
        .find(|entry| entry.light_index as usize == slot)?;
    let pixel = uv * floor.resolution as f32;
    let origin = UVec2::new(view.bounds[0], view.bounds[1]);
    let shadow = if entry.shadow_offset == 0 {
        1.0
    } else {
        ShadowTexels {
            words: &floor.triangles[entry.shadow_offset as usize..],
            width: view.bounds[2],
            origin,
            log_bits: log_bits(scene.lights[slot].shadow_bits_per_texel),
        }
        .shadow(pixel, quality)
    };
    let mut bounce = Vec3::ZERO;
    if entry.bounce_offset != 0 && quality != QUALITY_INTEGRATED {
        let flags = ChannelFlags::unpack(light.channel);
        bounce = BounceTexels {
            words: &floor.triangles[entry.bounce_offset as usize..],
            width: view.bounds[2],
            origin,
            bits: flags.bounce_bits_per_pixel,
            colored: flags.colored_bounce,
        }
        .bounce(pixel);
    }
    Some((shadow, bounce))
}

/// The floor color the shaders should produce at `world`, and its bounce light alone.
fn expected(scene: &BakedScene, lights: &PackedLights, world: Vec3) -> (Vec3, Vec3) {
    let quality = lights.globals.quality;
    let slots = lights
        .lights
        .iter()
        .take(lights.globals.baked_count as usize);
    slots
        .enumerate()
        .fold((Vec3::ZERO, Vec3::ZERO), |(sum, bounced), (slot, light)| {
            let Some((shadow, bounce)) = floor_terms(scene, light, slot, world, quality) else {
                return (sum, bounced);
            };
            let direct = Vec3::from(light.color)
                * mirror::radiance(light, world)
                * mirror::diffuse(light, world, Vec3::Y, true)
                * shadow;
            let distance_sqr = if mirror::is_distant(light) {
                0.0
            } else {
                Vec3::from(light.position).distance_squared(world)
            };
            let bounce =
                Vec3::from(light.bounce_color) * mirror::attenuation(light, distance_sqr) * bounce;
            (sum + direct + bounce, bounced + bounce)
        })
}

/// Pixels whose quarter-pixel neighbourhood stays off the wall.
fn checked(x: f32, y: f32) -> bool {
    let margin = 1.5 * EXTENT / SIZE as f32;
    let world = pixel_world(x, y);
    let (center, half) = WALL;
    (world.x - center.x).abs() > half.x + margin || (world.z - center.z).abs() > half.z + margin
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
    let (mut worst, mut failures, mut samples, mut red) = ((0.0, Vec2::ZERO), 0, 0, 0);
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
            let cpu: Vec<(Vec3, Vec3)> = around
                .iter()
                .map(|(dx, dy)| expected(&bake.scene, &lights, pixel_world(px + dx, py + dy)))
                .collect();
            let low = cpu.iter().fold(Vec3::MAX, |a, b| a.min(b.0));
            let high = cpu.iter().fold(Vec3::MIN, |a, b| a.max(b.0));
            let allowed = ABSOLUTE_TOLERANCE + RELATIVE_TOLERANCE * high.max_element();
            let error = (low - gpu).max(gpu - high).max(Vec3::ZERO).max_element();
            samples += 1;
            let bounce = cpu[0].1;
            red += usize::from(bounce.x > 0.02 && bounce.x > 4.0 * bounce.y);
            failures += usize::from(error > allowed);
            if error > worst.0 {
                worst = (error, Vec2::new(px, py));
            }
        }
    }
    let colored = bake.scene.settings.colored_bounce;
    let bounces = lights.globals.quality != QUALITY_INTEGRATED;
    let passed = failures == 0 && colored && (red >= MIN_RED || !bounces);
    if !passed && since < DEADLINE_SECONDS {
        return;
    }
    *done = true;
    info!(
        "{samples} samples ({red} with red bounce light), quality {}; largest difference {:.4} \
         at pixel {}",
        lights.globals.quality, worst.0, worst.1
    );
    if passed {
        info!("GPU bounce light matches the CPU mirror");
        exit.write(AppExit::Success);
    } else {
        error!("{failures} of {samples} samples differ from the CPU mirror (colored {colored})");
        exit.write(AppExit::error());
    }
}
