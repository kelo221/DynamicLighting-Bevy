//! Specular light of rectangle lights by linearly transformed cosines (Heitz, Dupuy, Hill and
//! Neubelt, "Real-Time Polygonal-Light Shading with Linearly Transformed Cosines", 2016): the
//! fitted GGX table and the polygon integral. CPU reference of `shaders/ltc.wesl` and of
//! `area_ltc_specular` in `shaders/area.wesl`.
//!
//! The table is the authors' fit (`LICENSES/LTC-BSD-2.txt`): 64 × 64 RGBA16F texels in two
//! layers, columns by perceptual roughness and rows by `sqrt(1 − n·v)`. Layer 0 holds the four
//! non-trivial entries of the inverse transform, layer 1 the BRDF's directional albedo without
//! (`x`) and with (`y`) Schlick's Fresnel factor.

use core::f32::consts::PI;

use bevy::math::{Vec2, Vec3, Vec4};

use super::area::{AreaShape, solid_angle};

/// Texels along each side of a table layer.
pub const LUT_SIZE: u32 = 64;
/// Both layers as little-endian RGBA16F, layer 0 first.
pub const LUT_BYTES: &[u8] = include_bytes!("ltc_lut.bin");
/// Below this solid angle the rectangle reflects from its representative point instead.
const MIN_SOLID_ANGLE: f32 = 1.0e-6;

/// One table lookup: `matrix` holds the inverse transform's entries `(m00, m20, m02, m22)`,
/// `albedo` the BRDF's directional albedo without and with the Fresnel factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LtcSample {
    pub matrix: Vec4,
    pub albedo: Vec4,
}

/// Texel `(x, y)` of table layer `layer`.
pub fn texel(layer: u32, x: u32, y: u32) -> Vec4 {
    let index = (((layer * LUT_SIZE + y) * LUT_SIZE + x) * 8) as usize;
    let bytes = &LUT_BYTES[index..index + 8];
    Vec4::from_array(core::array::from_fn(|c| {
        half::f16::from_le_bytes([bytes[2 * c], bytes[2 * c + 1]]).to_f32()
    }))
}

/// Where the table is read for `roughness` and the view cosine, mapped to texel centers.
pub fn lut_uv(roughness: f32, n_dot_v: f32) -> Vec2 {
    let size = LUT_SIZE as f32;
    let scale = (size - 1.0) / size;
    let bias = 0.5 / size;
    Vec2::new(roughness, (1.0 - n_dot_v.clamp(0.0, 1.0)).sqrt()) * scale + bias
}

/// A layer read at `uv` like a linear sampler with clamped edges.
pub fn sample_layer(layer: u32, uv: Vec2) -> Vec4 {
    let at = uv * LUT_SIZE as f32 - 0.5;
    let base = at.floor();
    let f = at - base;
    let last = LUT_SIZE as i32 - 1;
    let read = |dx: i32, dy: i32| {
        let x = (base.x as i32 + dx).clamp(0, last) as u32;
        let y = (base.y as i32 + dy).clamp(0, last) as u32;
        texel(layer, x, y)
    };
    let top = read(0, 0).lerp(read(1, 0), f.x);
    let bottom = read(0, 1).lerp(read(1, 1), f.x);
    top.lerp(bottom, f.y)
}

/// The table at `roughness` and the view cosine.
pub fn lookup(roughness: f32, n_dot_v: f32) -> LtcSample {
    let uv = lut_uv(roughness, n_dot_v);
    LtcSample {
        matrix: sample_layer(0, uv),
        albedo: sample_layer(1, uv),
    }
}

/// The matrix entries of the identity transform, which gives Lambert's form factor.
pub const IDENTITY: Vec4 = Vec4::new(1.0, 0.0, 0.0, 1.0);

/// The tangent of the shading frame: the view direction projected onto the surface, or any
/// tangent when looking straight down the normal (where the fit is symmetric).
fn tangent(normal: Vec3, view: Vec3) -> Vec3 {
    let projected = view - normal * normal.dot(view);
    if projected.length_squared() > 1.0e-12 {
        return projected.normalize();
    }
    let helper = if normal.x.abs() > 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    helper.cross(normal).normalize()
}

/// `direction` in the shading frame `(t1, normal × t1, normal)`, then through the inverse
/// transform.
fn transform(direction: Vec3, frame: [Vec3; 3], matrix: Vec4) -> Vec3 {
    let local = Vec3::new(
        direction.dot(frame[0]),
        direction.dot(frame[1]),
        direction.dot(frame[2]),
    );
    Vec3::new(
        matrix.x * local.x + matrix.z * local.z,
        local.y,
        matrix.y * local.x + matrix.w * local.z,
    )
}

/// The z part of the integral of the clamped-cosine distribution's edge term over the arc
/// between unit vectors `a` and `b`, with the authors' rational fit of `θ / sin θ` (the
/// `1 / 2π` included).
fn edge(a: Vec3, b: Vec3) -> f32 {
    let x = a.dot(b);
    let y = x.abs();
    let numerator = 0.854_398_5 + (0.496_515_5 + 0.014_520_6 * y) * y;
    let denominator = 3.417_594 + (4.161_672_4 + y) * y;
    let v = numerator / denominator;
    let theta_over_sin = if x > 0.0 {
        v
    } else {
        0.5 / (1.0 - x * x).max(1.0e-7).sqrt() - v
    };
    a.cross(b).z * theta_over_sin
}

/// The integral of the transformed cosine lobe over the quad `corners` (relative to the
/// surface, in order around it), seen from a surface with unit `normal` and `view`: 0 to 1.
pub fn quad_integral(corners: [Vec3; 4], normal: Vec3, view: Vec3, matrix: Vec4) -> f32 {
    let t1 = tangent(normal, view);
    let frame = [t1, normal.cross(t1), normal];
    let local = corners.map(|corner| transform(corner, frame, matrix));
    let mut clipped = [Vec3::ZERO; 5];
    let mut count = 0;
    for i in 0..4 {
        let (a, b) = (local[i], local[(i + 1) % 4]);
        if a.z >= 0.0 {
            clipped[count] = a;
            count += 1;
        }
        if (a.z >= 0.0) != (b.z >= 0.0) {
            clipped[count] = a.lerp(b, a.z / (a.z - b.z));
            count += 1;
        }
    }
    if count < 3 {
        return 0.0;
    }
    let unit = clipped.map(|v| v / v.length_squared().max(1.0e-20).sqrt());
    let sum: f32 = (0..count)
        .map(|i| edge(unit[i], unit[(i + 1) % count]))
        .sum();
    sum.abs()
}

/// The specular light a rectangle reflects at `world` per unit of light strength, with unit
/// `normal` and `view`, reflectance `f0` and perceptual `roughness`: π times the reflected
/// share over the solid angle, so a small rectangle reflects like a point light. `None` for
/// tubes and for rectangles below the smallest solid angle.
pub fn rectangle_specular(
    shape: &AreaShape,
    world: Vec3,
    normal: Vec3,
    view: Vec3,
    f0: Vec3,
    roughness: f32,
) -> Option<Vec3> {
    if !shape.rectangle {
        return None;
    }
    let solid = solid_angle(shape, world);
    if solid < MIN_SOLID_ANGLE {
        return None;
    }
    if shape.forward.dot(world - shape.center) <= 0.0 {
        return Some(Vec3::ZERO);
    }
    let table = lookup(roughness, normal.dot(view));
    let corners = shape.corners().map(|corner| corner - world);
    let integral = quad_integral(corners, normal, view, table.matrix);
    let albedo = f0 * table.albedo.x + (Vec3::ONE - f0) * table.albedo.y;
    Some(albedo * (PI * integral / solid))
}
