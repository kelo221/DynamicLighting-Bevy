//! A material that is not a `DynLitMaterial`, lit by realtime `DynamicLight`s: the hook for
//! extended materials, whose extension carries the bindings (`surface/external.rs`).
//! `ExternalMaterial` carries DynLight's scene-wide bindings at 120–132, adds
//! `external_shader_defs()` in `specialize` and calls `surface_light` from its fragment shader
//! (`external_material.wesl`). A point light and a tube light shine on a floor and a cube.
//!
//! `cargo run --example external_material -- --frames 120 --screenshot /tmp/external.png` exits
//! after 120 frames and saves the last ones as an image. With `--bake` the lights are baked
//! (`BakeStatic` meshes, in memory, `Preview`) and the material reads the baked shadows: it
//! implements `ExternalBakeMaterial` for its albedo, calls `external_specialize` and sets the
//! tag, primitive and lightmap UV in its fragment.

#[path = "support/lightmap_uvs.rs"]
mod lightmap_uvs;
#[path = "support/mod.rs"]
mod support;

use bevy::asset::uuid_handle;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::render::storage::ShaderBuffer;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::shader::{Shader, ShaderRef};
use dynlight::bake::{BakeFinished, BakePreset, BakeRequest};
use dynlight::baked_lighting::ActiveBake;
use dynlight::prelude::*;
use dynlight::surface::{ExternalBakeMaterial, external_specialize, external_vertex_shader};
use support::{argument, flag};

const SHADER: Handle<Shader> = uuid_handle!("7a1c3e55-0d42-4b8e-9f26-5c8e1a4d7b90");

/// Real frames before the screenshot is requested, then the frames it needs to be written.
const FRAMES_AFTER_SCREENSHOT: u32 = 5;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
struct ExternalMaterial {
    #[uniform(0)]
    base_color: LinearRgba,

    // DynLight's scene-wide bindings, the field block of `dynlight::surface::external`.
    #[storage(120, read_only)]
    dynlight_lights: Handle<ShaderBuffer>,
    #[texture(121, dimension = "2d_array")]
    #[sampler(122)]
    dynlight_cookies: Handle<Image>,
    #[storage(123, read_only)]
    dynlight_baked_triangles: Handle<ShaderBuffer>,
    #[storage(124, read_only)]
    dynlight_baked_aux: Handle<ShaderBuffer>,
    #[texture(125, dimension = "cube_array")]
    #[sampler(126)]
    dynlight_shadow_cubemaps: Handle<Image>,
    #[texture(127, dimension = "2d_array")]
    #[sampler(128)]
    dynlight_area_light_lut: Handle<Image>,
    #[storage(129, read_only)]
    dynlight_realtime_clusters: Handle<ShaderBuffer>,
    #[texture(130, dimension = "cube_array")]
    #[sampler(131)]
    dynlight_reflection: Handle<Image>,
    // Sampled with the shadow cubemaps' sampler (126).
    #[texture(132, dimension = "2d_array")]
    dynlight_sun_cascades: Handle<Image>,
}

impl ExternalMaterial {
    fn new(base_color: Color) -> Self {
        let bindings = DynLightBindings::default();
        Self {
            base_color: base_color.to_linear(),
            dynlight_lights: bindings.lights,
            dynlight_cookies: bindings.cookies,
            dynlight_baked_triangles: bindings.baked_triangles,
            dynlight_baked_aux: bindings.baked_aux,
            dynlight_shadow_cubemaps: bindings.shadow_cubemaps,
            dynlight_area_light_lut: bindings.area_light_lut,
            dynlight_realtime_clusters: bindings.realtime_clusters,
            dynlight_reflection: bindings.reflection,
            dynlight_sun_cascades: bindings.sun_cascades,
        }
    }
}

impl ExternalBakeMaterial for ExternalMaterial {
    /// Bounce light takes on the surface color.
    fn bake_albedo(&self) -> (LinearRgba, Option<Handle<Image>>) {
        (self.base_color, None)
    }
}

impl Material for ExternalMaterial {
    fn vertex_shader() -> ShaderRef {
        external_vertex_shader()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        external_specialize(pipeline, descriptor, layout)
    }
}

#[derive(Resource)]
struct Options {
    frames: Option<u32>,
    screenshot: Option<String>,
    bake: bool,
}

fn main() -> AppExit {
    let options = Options {
        frames: argument("--frames").and_then(|value| value.parse().ok()),
        screenshot: argument("--screenshot"),
        bake: flag("--bake"),
    };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(support::window("DynLight external material")),
            ..default()
        }))
        .add_plugins((
            DynamicLightingPlugin,
            MaterialPlugin::<ExternalMaterial>::default(),
            ExternalLightingPlugin::<ExternalMaterial>::default(),
        ))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
        .insert_resource(options)
        .add_systems(Startup, spawn_scene)
        .add_systems(Update, (request_bake, apply_bake))
        .add_systems(Last, finish)
        .run()
}

fn spawn_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ExternalMaterial>>,
    mut shaders: ResMut<Assets<Shader>>,
    options: Res<Options>,
) {
    let realtime = !options.bake;
    let source = include_str!("external_material.wesl");
    let path = "embedded://external_material/external_material.wesl";
    shaders
        .insert(SHADER.id(), Shader::from_wesl(source, path))
        .expect("UUID asset ids always insert");

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 4.5, 7.5).looking_at(Vec3::new(0.0, 0.4, 0.0), Vec3::Y),
    ));
    commands.spawn((
        Name::new("Floor"),
        BakeStatic,
        Mesh3d(meshes.add(lightmap_uvs::with_uv0_as_uv1(
            Plane3d::default().mesh().size(16.0, 16.0).build(),
        ))),
        MeshMaterial3d(materials.add(ExternalMaterial::new(Color::srgb(0.8, 0.8, 0.8)))),
    ));
    commands.spawn((
        Name::new("Cube"),
        BakeStatic,
        Mesh3d(meshes.add(lightmap_uvs::box_with_lightmap_uvs(Cuboid::new(
            1.4, 1.4, 1.4,
        )))),
        MeshMaterial3d(materials.add(ExternalMaterial::new(Color::srgb(0.85, 0.6, 0.35)))),
        Transform::from_xyz(0.0, 0.7, 0.0).with_rotation(Quat::from_rotation_y(0.5)),
    ));
    commands.spawn((
        Name::new("Point light"),
        DynamicLight {
            color: Color::srgb(1.0, 0.7, 0.4),
            radius: 8.0,
            intensity: 2.0,
            realtime,
            ..default()
        },
        Transform::from_xyz(-3.0, 2.0, 1.5),
    ));
    commands.spawn((
        Name::new("Tube light"),
        DynamicLight {
            color: Color::srgb(0.6, 0.8, 1.0),
            radius: 8.0,
            intensity: 2.0,
            realtime,
            ..DynamicLight::tube(1.2)
        },
        Transform::from_xyz(3.0, 2.0, 1.0),
    ));
    support::status("A custom material lit by a realtime point light and a tube light");
}

/// With `--bake`: bakes once the scene's transforms and derived albedos are up to date.
fn request_bake(
    options: Res<Options>,
    mut frames: Local<u32>,
    mut requests: MessageWriter<BakeRequest>,
) {
    *frames += 1;
    if options.bake && *frames == 3 {
        requests.write(BakeRequest {
            output: None,
            preset: BakePreset::Preview,
        });
    }
}

/// Applies the finished bake; the meshes get their tags and the fragments read the shadows.
fn apply_bake(
    mut finished: MessageReader<BakeFinished>,
    mut active: ResMut<ActiveBake>,
    mut exit: MessageWriter<AppExit>,
) {
    for bake in finished.read() {
        match &bake.result {
            Ok(report) => active.0 = Some(report.handle.clone()),
            Err(failure) => {
                error!("bake failed: {failure}");
                exit.write(AppExit::error());
            }
        }
    }
}

/// With `--frames N`: requests the screenshot five frames before the end and exits at N.
fn finish(
    mut commands: Commands,
    options: Res<Options>,
    mut frame: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(frames) = options.frames else {
        return;
    };
    *frame += 1;
    if let Some(path) = &options.screenshot
        && *frame == frames.saturating_sub(FRAMES_AFTER_SCREENSHOT).max(1)
    {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path.clone()));
    }
    if *frame >= frames {
        exit.write(AppExit::Success);
    }
}
