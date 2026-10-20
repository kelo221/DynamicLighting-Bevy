//! Captures one probe at a time: six 90° cameras render its faces into float images for a few
//! frames, the faces are read back, and a background task prefilters them.

use core::f32::consts::FRAC_PI_2;
use std::sync::{Arc, Mutex, PoisonError};

use bevy::camera::{Hdr, ImageRenderTarget, RenderTarget};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::gpu_readback::{ReadbackComplete, ReadbackOnce};
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on};

use super::assemble::{ProbeLighting, probe_lighting, read_face};
use super::systems::{Captured, Probes};
use super::{ReflectionProbe, ReflectionProbeStatus, WaitingPipelines};
use crate::environment::SkyStatus;
use crate::light::CaptureViewpoint;
use crate::shared::cube::{CUBE_FACE_COUNT, face_orientation};
use crate::shared::{LitShaderFeatures, SceneLoading};

/// Frames in a row without a compiling pipeline before a capture starts.
const IDLE_FRAMES: u32 = 8;
/// Frames the capture cameras render before their faces are read: the realtime lights are
/// chosen around the probe from the next frame on, and new views may compile pipelines.
const WARMUP_FRAMES: u32 = 4;
/// A capture whose faces haven't come back this many frames after they were requested is given
/// up. Frames before that don't count: the capture waits for pipelines to compile.
const TIMEOUT_FRAMES: u32 = 600;
/// Below every ordinary camera, so none of them becomes the viewer.
const CAMERA_ORDER: isize = -10_000;
const NEAR_PLANE: f32 = 0.02;

type Faces = Arc<Mutex<Vec<Option<Vec<u8>>>>>;

/// A probe whose faces are being rendered.
pub(super) struct Rig {
    probe: Entity,
    capture: Vec3,
    resolution: u32,
    cameras: Vec<Entity>,
    images: Vec<Handle<Image>>,
    frames: u32,
    /// Frames since the readbacks were requested.
    waited: u32,
    /// Set once the readbacks are requested.
    readbacks: Vec<Entity>,
    faces: Faces,
}

/// Where the capture stands.
#[derive(Default)]
pub(super) enum Stage {
    #[default]
    Idle,
    Rendering(Rig),
    Filtering {
        probe: Entity,
        capture: Vec3,
        task: Task<Option<ProbeLighting>>,
    },
}

fn face_image(resolution: u32) -> Image {
    let mut image =
        Image::new_target_texture(resolution, resolution, TextureFormat::Rgba16Float, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image
}

fn face_camera(image: Handle<Image>, face: u32, capture: Vec3) -> impl Bundle {
    let (forward, up) = face_orientation(face);
    (
        Name::new("dynlight probe capture"),
        Camera3d::default(),
        Camera {
            order: CAMERA_ORDER - face as isize,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        RenderTarget::Image(ImageRenderTarget::from(image)),
        Projection::Perspective(PerspectiveProjection {
            fov: FRAC_PI_2,
            aspect_ratio: 1.0,
            near: NEAR_PLANE,
            ..default()
        }),
        Transform::from_translation(capture).looking_to(forward, up),
        Hdr,
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
    )
}

fn start(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    probe: Entity,
    settings: &ReflectionProbe,
    transform: &GlobalTransform,
) -> Rig {
    let capture = transform.translation() + settings.capture_offset;
    let resolution = settings.resolution.clamp(16, 2048);
    let images: Vec<Handle<Image>> = (0..CUBE_FACE_COUNT)
        .map(|_| images.add(face_image(resolution)))
        .collect();
    let cameras = (0..CUBE_FACE_COUNT)
        .map(|face| {
            let image = images[face as usize].clone();
            commands.spawn(face_camera(image, face, capture)).id()
        })
        .collect();
    Rig {
        probe,
        capture,
        resolution,
        cameras,
        images,
        frames: 0,
        waited: 0,
        readbacks: Vec::new(),
        faces: Arc::new(Mutex::new(vec![None; CUBE_FACE_COUNT as usize])),
    }
}

fn finish(commands: &mut Commands, images: &mut Assets<Image>, rig: &Rig) {
    for &entity in rig.cameras.iter().chain(&rig.readbacks) {
        commands.entity(entity).despawn();
    }
    for image in &rig.images {
        images.remove(image);
    }
}

/// Moves the capture on by a frame.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_captures(
    mut commands: Commands,
    mut probes: ResMut<Probes>,
    query: Query<(&ReflectionProbe, &GlobalTransform)>,
    mut images: ResMut<Assets<Image>>,
    waiting: Res<WaitingPipelines>,
    (sky, features, loading): (
        Option<Res<SkyStatus>>,
        Res<LitShaderFeatures>,
        Res<SceneLoading>,
    ),
    mut viewpoint: ResMut<CaptureViewpoint>,
    mut status: ResMut<ReflectionProbeStatus>,
) {
    let probes = &mut *probes;
    let compiling = waiting.compiling();
    probes.idle_frames = if compiling { 0 } else { probes.idle_frames + 1 };
    let ready = probes.idle_frames >= IDLE_FRAMES
        && !sky.is_some_and(|sky| sky.building)
        && features.0.is_some()
        && !loading.0;
    probes.stage = match core::mem::take(&mut probes.stage) {
        Stage::Idle => {
            // Lowest entity first: only when each probe is captured depends on it.
            let next = ready.then(|| probes.stale.iter().copied().min()).flatten();
            match next.map(|probe| (probe, query.get(probe))) {
                Some((probe, Ok((settings, transform)))) => {
                    probes.stale.remove(&probe);
                    let rig = start(&mut commands, &mut images, probe, settings, transform);
                    viewpoint.0 = Some(rig.capture);
                    Stage::Rendering(rig)
                }
                Some((probe, Err(_))) => {
                    probes.stale.remove(&probe);
                    Stage::Idle
                }
                None => Stage::Idle,
            }
        }
        Stage::Rendering(mut rig) => {
            rig.frames += 1;
            rig.waited += u32::from(!rig.readbacks.is_empty());
            let faces: Option<Vec<Vec<u8>>> = rig
                .faces
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .cloned()
                .collect();
            if query.get(rig.probe).is_err() || rig.waited > TIMEOUT_FRAMES {
                if rig.waited > TIMEOUT_FRAMES {
                    warn!("dynlight reflection probes: a capture timed out; trying again later");
                    probes.stale.insert(rig.probe);
                }
                finish(&mut commands, &mut images, &rig);
                viewpoint.0 = None;
                Stage::Idle
            } else if let Some(faces) = faces {
                finish(&mut commands, &mut images, &rig);
                viewpoint.0 = None;
                let resolution = rig.resolution;
                let task = AsyncComputeTaskPool::get().spawn(async move {
                    let faces: Option<Vec<_>> = faces
                        .iter()
                        .map(|bytes| read_face(bytes, resolution))
                        .collect();
                    faces.map(|faces| probe_lighting(&faces, resolution))
                });
                Stage::Filtering {
                    probe: rig.probe,
                    capture: rig.capture,
                    task,
                }
            } else {
                if rig.readbacks.is_empty() && rig.frames >= WARMUP_FRAMES && !compiling {
                    for (face, image) in rig.images.iter().enumerate() {
                        let faces = rig.faces.clone();
                        let readback = commands
                            .spawn(ReadbackOnce::texture(image.clone()))
                            .observe(move |readback: On<ReadbackComplete>| {
                                let mut faces =
                                    faces.lock().unwrap_or_else(PoisonError::into_inner);
                                faces[face] = Some(readback.data.clone());
                            })
                            .id();
                        rig.readbacks.push(readback);
                    }
                }
                Stage::Rendering(rig)
            }
        }
        Stage::Filtering {
            probe,
            capture,
            task,
        } => {
            if !task.is_finished() {
                Stage::Filtering {
                    probe,
                    capture,
                    task,
                }
            } else {
                match block_on(task) {
                    Some(lighting) if query.contains(probe) => {
                        let lighting = Arc::new(lighting);
                        probes.results.insert(probe, Captured { lighting, capture });
                        probes.rebuild = true;
                    }
                    Some(_) => {}
                    None => warn!("dynlight reflection probes: a captured face was cut short"),
                }
                Stage::Idle
            }
        }
    };
    let busy = usize::from(!matches!(probes.stage, Stage::Idle));
    status.set_if_neq(ReflectionProbeStatus {
        pending: probes.stale.len() + busy,
        captured: probes.results.len(),
    });
}
