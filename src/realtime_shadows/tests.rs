//! Caster tracking in an app: which edits report a caster to the cube planning, and which asset
//! events leave the cached faces alone.

use bevy::asset::{AssetEventSystems, AssetPlugin, RenderAssetUsages};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
use bevy::image::ImageSampler;
use bevy::mesh::PrimitiveTopology;
use bevy::pbr::{Material, MeshMaterial3d};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;

use super::materials::{CasterMaterial, touch_casters};
use super::tracking::{CasterBounds, CasterChanges, ShadowFeedback, TouchedCasters, track_casters};

#[derive(Asset, TypePath, AsBindGroup, Clone)]
struct Masked {
    #[texture(0)]
    #[sampler(1)]
    texture: Option<Handle<Image>>,
    cutoff: f32,
}

impl Material for Masked {}

impl CasterMaterial for Masked {
    fn visit_textures(&self, visit: &mut dyn FnMut(AssetId<Image>)) {
        self.texture.iter().for_each(|texture| visit(texture.id()));
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Image>()
        .init_asset::<Mesh>()
        .init_asset::<Masked>()
        .init_resource::<TouchedCasters>()
        .init_resource::<CasterBounds>()
        .init_resource::<CasterChanges>()
        .init_resource::<ShadowFeedback>()
        .add_systems(
            PostUpdate,
            (touch_casters::<Masked>, track_casters)
                .chain()
                .after(AssetEventSystems),
        );
    app
}

fn mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
}

/// Edits an image, as a growing font atlas or a streamed texture does.
fn edit(app: &mut App, image: &Handle<Image>) {
    let mut images = app.world_mut().resource_mut::<Assets<Image>>();
    if let Some(mut image) = images.get_mut(image) {
        image.sampler = ImageSampler::nearest();
    }
}

/// Runs a frame; whether any caster was reported, and whether every face renders.
fn frame(app: &mut App) -> (bool, bool) {
    app.update();
    let changes = app.world().resource::<CasterChanges>();
    (!changes.changes.is_empty(), changes.settle > 0)
}

/// Runs frames until nothing is reported any more.
fn settle(app: &mut App) {
    for _ in 0..10 {
        if frame(app) == (false, false) {
            return;
        }
    }
    panic!("the caster is still reported");
}

fn spawn_caster(app: &mut App) -> (Entity, Handle<Masked>, Handle<Image>) {
    let world = app.world_mut();
    let texture = world.resource_mut::<Assets<Image>>().add(Image::default());
    let material = world.resource_mut::<Assets<Masked>>().add(Masked {
        texture: Some(texture.clone()),
        cutoff: 0.5,
    });
    let mesh = world.resource_mut::<Assets<Mesh>>().add(mesh());
    let caster = world
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material.clone()),
            GlobalTransform::default(),
            Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
            InheritedVisibility::VISIBLE,
        ))
        .id();
    (caster, material, texture)
}

#[test]
fn unrelated_assets_leave_the_faces_alone() {
    let mut app = app();
    spawn_caster(&mut app);
    assert!(frame(&mut app).0, "a new caster is reported");
    settle(&mut app);

    let world = app.world_mut();
    let atlas = world.resource_mut::<Assets<Image>>().add(Image::default());
    world.resource_mut::<Assets<Mesh>>().add(mesh());
    assert_eq!(frame(&mut app), (false, false));
    edit(&mut app, &atlas);
    assert_eq!(frame(&mut app), (false, false), "a growing font atlas");
}

#[test]
fn material_edits_and_texture_loads_report_the_caster() {
    let mut app = app();
    let (caster, material, texture) = spawn_caster(&mut app);
    settle(&mut app);

    if let Some(mut edited) = app
        .world_mut()
        .resource_mut::<Assets<Masked>>()
        .get_mut(&material)
    {
        edited.cutoff = 0.8;
    }
    assert_eq!(frame(&mut app), (true, false), "a new alpha cutoff");
    settle(&mut app);

    edit(&mut app, &texture);
    assert_eq!(frame(&mut app), (true, false), "its texture changed");
    settle(&mut app);

    let world = app.world_mut();
    let other = world.resource_mut::<Assets<Masked>>().add(Masked {
        texture: None,
        cutoff: 0.5,
    });
    world.entity_mut(caster).insert(MeshMaterial3d(other));
    assert_eq!(frame(&mut app), (true, false), "another material");
    settle(&mut app);
}

#[test]
fn render_layer_changes_report_the_caster() {
    let mut app = app();
    let (caster, ..) = spawn_caster(&mut app);
    settle(&mut app);
    app.world_mut()
        .entity_mut(caster)
        .insert(RenderLayers::layer(1));
    assert_eq!(frame(&mut app), (true, false), "left the cameras' layer");
    settle(&mut app);
    app.world_mut().entity_mut(caster).remove::<RenderLayers>();
    assert_eq!(frame(&mut app), (true, false), "back on the default layer");
    settle(&mut app);
}
