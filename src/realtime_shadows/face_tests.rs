//! Shadow faces in an app: the casters a face lists are marked visible even when no camera sees
//! them, and only faces rendering this frame reach the render world as views.

use bevy::asset::{AssetPlugin, RenderAssetUsages};
use bevy::camera::CameraProjection;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::{VisibilityPlugin, VisibilitySystems};
use bevy::core_pipeline::schedule::RootNonCameraView;
use bevy::ecs::system::RunSystemOnce;
use bevy::mesh::PrimitiveTopology;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use bevy::render::MainWorld;
use bevy::render::sync_world::{MainEntity, RenderEntity};
use bevy::render::view::{ExtractedView, NoIndirectDrawing};

use super::cascades::{CascadeViewer, fit_cascades};
use super::casters::ActiveCube;
use super::extract::{ShadowFaceView, extract_shadow_faces};
use super::slots::{ALL_FACES, CubePose};
use super::sun::SunCascadeView;
use super::tracking::ShadowTarget;
use super::views::{ShadowFace, ShadowFaceUniform, ShadowFaces, cull_casters};

const POSE: CubePose = CubePose {
    position: Vec3::ZERO,
    radius: 4.0,
    moving_only: false,
    blur: 1.5,
};

fn face(layer: u32, renders: bool) -> ShadowFace {
    ShadowFace {
        layer,
        pose: POSE,
        renders,
        casters: Vec::new(),
    }
}

fn spawn_caster(app: &mut App, mesh: &Handle<Mesh>, position: Vec3) -> Entity {
    app.world_mut()
        .spawn((
            Mesh3d(mesh.clone()),
            Transform::from_translation(position),
            Aabb::from_min_max(Vec3::splat(-0.5), Vec3::splat(0.5)),
        ))
        .id()
}

#[test]
fn listed_casters_are_visible_without_a_camera() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        TransformPlugin,
        VisibilityPlugin,
    ))
    .init_asset::<Mesh>()
    .init_asset::<SkinnedMeshInverseBindposes>()
    .init_resource::<ShadowFaces>()
    .add_systems(
        PostUpdate,
        cull_casters.in_set(VisibilitySystems::CheckVisibility),
    );
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        ));
    // In front of face 0 (+X), and far beyond the light's radius.
    let near = spawn_caster(&mut app, &mesh, Vec3::new(2.0, 0.0, 0.0));
    let far = spawn_caster(&mut app, &mesh, Vec3::new(40.0, 0.0, 0.0));
    let faces: Vec<Entity> = (0..6)
        .map(|layer| app.world_mut().spawn(face(layer, true)).id())
        .collect();
    let mut placed = app.world_mut().resource_mut::<ShadowFaces>();
    placed.faces = faces.clone();
    placed.active = vec![ActiveCube {
        slot: 0,
        pose: POSE,
        faces: ALL_FACES,
    }];

    app.update();
    let world = app.world();
    let visible = |entity| world.get::<ViewVisibility>(entity).unwrap().get();
    assert!(visible(near), "no camera sees it, but the light does");
    assert!(!visible(far), "out of the light's reach");
    let listed = |layer: usize| {
        world
            .get::<ShadowFace>(faces[layer])
            .unwrap()
            .casters
            .clone()
    };
    assert_eq!(listed(0), [near]);
    assert!(listed(1).is_empty(), "face 1 looks away from it");
}

/// The view is drawn directly and by the shadow driver alone, not by a schedule of its own.
fn has_view(render: &World, entity: Entity) -> bool {
    let entity = render.entity(entity);
    entity.contains::<ExtractedView>()
        && !entity.contains::<RootNonCameraView>()
        && entity.contains::<NoIndirectDrawing>()
        && entity.contains::<ShadowFaceView>()
}

#[test]
fn only_rendering_faces_carry_a_view() {
    let mut render = World::new();
    let (first, second) = (render.spawn_empty().id(), render.spawn_empty().id());
    let mut main = MainWorld::default();
    let caster = main.spawn_empty().id();
    let mut drawing = face(0, true);
    drawing.casters.push(caster);
    main.spawn((drawing, RenderEntity::from(first)));
    let stopping = main.spawn((face(1, true), RenderEntity::from(second))).id();
    render.insert_resource(main);

    render.run_system_once(extract_shadow_faces).unwrap();
    assert!(has_view(&render, first) && has_view(&render, second));
    let view = render.get::<ShadowFaceView>(first).unwrap();
    assert_eq!(view.target, ShadowTarget::Face(0));
    // Meshes are keyed by main entity in the render world.
    assert_eq!(
        view.casters,
        [(Entity::PLACEHOLDER, MainEntity::from(caster))]
    );

    let mut main = render.resource_mut::<MainWorld>();
    main.get_mut::<ShadowFace>(stopping).unwrap().renders = false;
    render.run_system_once(extract_shadow_faces).unwrap();
    assert!(has_view(&render, first));
    let stopped = render.entity(second);
    assert!(!stopped.contains::<ExtractedView>() && !stopped.contains::<ShadowFaceView>());
}

#[test]
fn rendering_cascades_carry_an_orthographic_view() {
    let mut render = World::new();
    let (first, second) = (render.spawn_empty().id(), render.spawn_empty().id());
    let mut main = MainWorld::default();
    let viewer = CascadeViewer {
        world_from_view: Mat4::IDENTITY,
        clip_from_view: PerspectiveProjection::default().get_clip_from_view(),
    };
    let poses = fit_cascades(&viewer, Vec3::Y, 50.0, false, &[]);
    let cascade = |index: usize, renders| SunCascadeView {
        index: index as u32,
        pose: Some(poses[index]),
        renders,
        casters: Vec::new(),
    };
    main.spawn((cascade(2, true), RenderEntity::from(first)));
    main.spawn((cascade(1, false), RenderEntity::from(second)));
    render.insert_resource(main);

    render.run_system_once(extract_shadow_faces).unwrap();
    assert!(has_view(&render, first) && !has_view(&render, second));
    let view = render.get::<ShadowFaceView>(first).unwrap();
    assert_eq!(view.target, ShadowTarget::Cascade(2));
    let uniform = render.get::<ShadowFaceUniform>(first).unwrap();
    assert_eq!((uniform.layer, uniform.orthographic), (2, 1));
    let extracted = render.get::<ExtractedView>(first).unwrap();
    assert_eq!(extracted.clip_from_view, poses[2].view().1);
}
