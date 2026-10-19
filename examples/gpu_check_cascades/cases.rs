//! The synthetic cascades and moments of `gpu_check_cascades`, the cases it steps through, and
//! which way through `sun_shadow` a floor point goes, so the check knows every way was tested.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::math::{Vec2, Vec3};
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use dynlight::realtime_shadows::sun_mirror::{
    SUN_CASCADE_BLEND, SUN_CASCADE_MARGIN, cascade_point, sun_shadow,
};
use dynlight::shared::{GpuSunCascade, GpuSunShadow, SUN_CASCADE_COUNT};

/// Moment texels per side of each cascade.
pub const MOMENTS_SIZE: u32 = 64;

/// Each cascade's square on the floor: center (x, z) and half side, in meters. Cascade 1 holds
/// only part of cascade 0's rim, so a point fading out of cascade 0 can miss the next cascade;
/// cascade 2 holds the others.
pub const SQUARES: [(Vec2, f32); SUN_CASCADE_COUNT] = [
    (Vec2::new(0.0, -1.5), 2.0),
    (Vec2::new(0.0, 1.0), 3.0),
    (Vec2::new(0.0, 0.0), 4.6),
];

/// The depth row: 0.6 on the floor, a little less above it.
const DEPTH: [f32; 4] = [0.0, -0.1, 0.0, 0.6];

/// Moments (E[x], E[x²]) of each cascade's near (v < 0.5) and far half: a caster at depth 0.4
/// with different variances, or nothing (1, 1), so every half lets a different share through.
const MOMENTS: [[(f32, f32); 2]; SUN_CASCADE_COUNT] = [
    [(0.4, 0.16), (0.4, 0.2)],
    [(0.4, 0.28), (1.0, 1.0)],
    [(0.4, 0.224), (0.4, 0.18)],
];

#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub count: u32,
    pub moving_only: bool,
}

impl Case {
    const fn new(count: u32, moving_only: bool) -> Self {
        Self { count, moving_only }
    }
}

pub const CASES: [Case; 5] = [
    Case::new(0, false),
    Case::new(1, false),
    Case::new(2, false),
    Case::new(3, false),
    Case::new(3, true),
];

/// A half float's value, as the moments texture holds it.
fn half(value: f32) -> f32 {
    half::f16::from_f32(value).to_f32()
}

/// The moments cascade `index` holds at `uv`, away from the line between its halves.
pub fn moments(index: usize, uv: Vec2) -> Vec2 {
    let (mean, square) = MOMENTS[index][usize::from(uv.y >= 0.5)];
    Vec2::new(half(mean), half(square))
}

fn cascade(index: usize) -> GpuSunCascade {
    let (center, half_side) = SQUARES[index];
    let scale = 0.5 / half_side;
    GpuSunCascade {
        u: [scale, 0.0, 0.0, 0.5 - center.x * scale],
        v: [0.0, 0.0, scale, 0.5 - center.y * scale],
        depth: DEPTH,
        texel_size: 2.0 * half_side / MOMENTS_SIZE as f32,
        depth_bias: 0.001,
        _padding: [0.0; 2],
    }
}

/// The case's cascades. All three are filled in whatever the count, so a shader reading past
/// it would show.
pub fn block(case: Case) -> GpuSunShadow {
    GpuSunShadow {
        cascades: core::array::from_fn(cascade),
        count: case.count,
        flags: if case.moving_only {
            GpuSunShadow::FLAG_MOVING_ONLY
        } else {
            0
        },
        _padding: [0; 2],
    }
}

/// The moments array: one layer per cascade, its two halves along v.
pub fn moments_image() -> Image {
    let side = MOMENTS_SIZE;
    let mut data = Vec::with_capacity((side * side * SUN_CASCADE_COUNT as u32 * 8) as usize);
    for layer in 0..SUN_CASCADE_COUNT {
        for row in 0..side {
            let value = moments(layer, Vec2::new(0.5, (row as f32 + 0.5) / side as f32));
            for _ in 0..side {
                for channel in [value.x, value.y, 0.0, 1.0] {
                    data.extend(half::f16::from_f32(channel).to_le_bytes());
                }
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: SUN_CASCADE_COUNT as u32,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..Default::default()
    });
    image
}

/// Floor points whose lookups in every cascade stay a texel away from its halves' line, where
/// the filtered moments aren't one half's.
pub fn clear_of_splits(world: Vec3, margin: f32) -> bool {
    SQUARES.iter().all(|(center, half_side)| {
        let texel = 2.0 * half_side / MOMENTS_SIZE as f32;
        (world.z - center.y).abs() > texel + margin
    })
}

/// The share of the sun that reaches the floor at `world`, by the CPU mirror.
pub fn expected(case: Case, world: Vec3, cosine: f32, static_receiver: bool) -> f32 {
    sun_shadow(
        &block(case),
        world,
        Vec3::Y,
        cosine,
        static_receiver,
        moments,
    )
}

/// Which way through `sun_shadow` a floor point goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Way {
    /// Inside cascade `n`, short of its blend band.
    Core(usize),
    /// In cascade `n`'s blend band, inside the next cascade.
    IntoNext(usize),
    /// In cascade `n`'s blend band, outside the next cascade.
    NextMisses(usize),
    /// In the blend band of the last cascade.
    IntoNone(usize),
    /// Outside every cascade.
    Outside,
}

pub fn way(case: Case, world: Vec3) -> Way {
    let block = block(case);
    let count = (case.count as usize).min(SUN_CASCADE_COUNT);
    let inner = 1.0 - SUN_CASCADE_MARGIN;
    let rim = |index: usize| cascade_point(&block.cascades[index], world, Vec3::Y).rim;
    let Some(index) = (0..count).find(|&index| rim(index) < inner) else {
        return Way::Outside;
    };
    if rim(index) <= inner - SUN_CASCADE_BLEND {
        Way::Core(index)
    } else if index + 1 == count {
        Way::IntoNone(index)
    } else if rim(index + 1) < inner {
        Way::IntoNext(index)
    } else {
        Way::NextMisses(index)
    }
}

/// The ways a case must take on each floor.
pub fn required(case: Case) -> Vec<Way> {
    use Way::*;
    match case.count {
        0 => vec![Outside],
        1 => vec![Core(0), IntoNone(0), Outside],
        2 => vec![
            Core(0),
            IntoNext(0),
            NextMisses(0),
            Core(1),
            IntoNone(1),
            Outside,
        ],
        _ => vec![
            Core(0),
            IntoNext(0),
            NextMisses(0),
            Core(1),
            IntoNext(1),
            Core(2),
            IntoNone(2),
            Outside,
        ],
    }
}
