//! What the baker consumes: static meshes in world space and baked lights, plus options.
//!
//! Plain data with no Bevy ECS types, so bakes can run headless and in tests.

use std::fmt;

use bevy::math::{Vec2, Vec3};

use crate::light::{BounceCompression, DynamicLight, Illumination, LightShape, Transparency};
use crate::shared::{BakeId, DynamicGeometryMode};

use super::format::MAX_SUN_MAP_RESOLUTION;
use super::settings::BakeSettings;

/// Everything a bake needs. Order doesn't matter: meshes and lights are sorted by [`BakeId`].
#[derive(Clone, Debug, Default)]
pub struct BakeInput {
    pub meshes: Vec<BakeMesh>,
    pub lights: Vec<BakeLight>,
}

/// One static mesh, already transformed to world space.
#[derive(Clone, Debug)]
pub struct BakeMesh {
    pub id: BakeId,
    /// [`crate::shared::mesh_content_hash`] of the local mesh data, stored for stale detection.
    pub content_hash: u64,
    pub positions: Vec<Vec3>,
    /// World-space vertex normals; empty means flat (face) normals.
    pub normals: Vec<Vec3>,
    /// Triangle list indices. Triangle `i` is primitive index `i` at runtime, so this must be
    /// the index order the mesh is drawn with.
    pub indices: Vec<u32>,
    /// Shadow map (UV1) coordinates, one per vertex. Use UV0 when the mesh has no UV1.
    pub uv1: Vec<Vec2>,
    /// Texture coordinates for [`Self::alpha`].
    pub uv0: Option<Vec<Vec2>>,
    /// Base color alpha for transparent shadows; needs [`Self::uv0`].
    pub alpha: Option<AlphaTexture>,
    /// Whether the mesh casts shadows. Meshes that don't are still baked as receivers, but every
    /// ray passes through them.
    pub occludes: bool,
    /// The mean linear color the mesh reflects (white by default). Only colored bounce reads it.
    pub albedo: Vec3,
}

impl Default for BakeMesh {
    fn default() -> Self {
        Self {
            id: BakeId(0),
            content_hash: 0,
            positions: Vec::new(),
            normals: Vec::new(),
            indices: Vec::new(),
            uv1: Vec::new(),
            uv0: None,
            alpha: None,
            occludes: true,
            albedo: Vec3::ONE,
        }
    }
}

/// An RGBA8 base color texture whose alpha decides where light passes through.
#[derive(Clone, Debug, PartialEq)]
pub struct AlphaTexture {
    pub width: u32,
    pub height: u32,
    /// `width · height` RGBA8 texels, rows top to bottom.
    pub rgba8: Vec<u8>,
    /// Texels with alpha below this let light through. Henry uses 0.5.
    pub cutoff: f32,
}

impl AlphaTexture {
    pub const DEFAULT_CUTOFF: f32 = 0.5;

    /// Whether the texel at `uv` (nearest, repeating) blocks light.
    pub fn is_opaque(&self, uv: Vec2) -> bool {
        if self.width == 0 || self.height == 0 {
            return true;
        }
        let x = wrap(uv.x, self.width);
        let y = wrap(uv.y, self.height);
        let alpha = self.rgba8[(y * self.width + x) as usize * 4 + 3];
        f32::from(alpha) / 255.0 >= self.cutoff
    }
}

fn wrap(coordinate: f32, size: u32) -> u32 {
    let texel = (coordinate.rem_euclid(1.0) * size as f32).floor();
    if texel.is_finite() {
        (texel as u32).min(size - 1)
    } else {
        0
    }
}

/// One baked (non-realtime) light.
#[derive(Clone, Debug, PartialEq)]
pub struct BakeLight {
    pub id: BakeId,
    pub position: Vec3,
    pub radius: f32,
    pub illumination: Illumination,
    pub transparency: Transparency,
    pub bounce: BakeBounce,
    /// What shadow rays are traced to: the position for point-like lights, the shape of a
    /// rectangle or tube light centered at [`Self::position`], a sun's disk, or the sky. Suns and
    /// skies ignore the position and radius.
    pub shape: LightShape,
}

/// The bounce settings the tracer reads, from [`crate::light::LightBounce`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakeBounce {
    /// Samples per texel, at least 2.
    pub samples: u32,
    pub intensity: f32,
    /// `None` uses [`BakeOptions::bounce_compression`].
    pub compression: Option<BounceCompression>,
}

impl Default for BakeBounce {
    fn default() -> Self {
        Self {
            samples: 32,
            intensity: 1.0,
            compression: None,
        }
    }
}

/// How finely and what to bake.
#[derive(Clone, Debug, PartialEq)]
pub struct BakeOptions {
    /// Shadow texels per meter.
    pub pixel_density: f32,
    /// Largest shadow map side per mesh.
    pub max_lightmap_size: u32,
    /// Bounce precision for lights without their own.
    pub bounce_compression: BounceCompression,
    /// Decides whether distance cubes are baked, and their bias.
    pub dynamic_geometry: DynamicGeometryMode,
    /// Skips bounce lighting even for `SingleBounce` lights (the Preview preset).
    pub skip_bounce: bool,
    /// Side of the per-light photon cube faces used for bounce. Henry renders 1024².
    pub photon_cube_resolution: u32,
    /// Rays per texel to an area light where its corners and center disagree.
    pub area_light_samples: u32,
    /// Of those, the rays every texel takes before it may count as fully lit or shadowed.
    pub area_light_min_samples: u32,
    /// Side of each baked sun's sun map in texels.
    pub sun_map_resolution: u32,
    /// Rays per texel and per probe direction to a baked sky.
    pub sky_samples: u32,
    /// Meters between sky probes, before the grid is capped at
    /// [`MAX_SKY_PROBES`](super::MAX_SKY_PROBES).
    pub sky_probe_spacing: f32,
    /// Bounce light takes on the reflecting surfaces' [`BakeMesh::albedo`].
    pub colored_bounce: bool,
    /// Seed for every random choice (bounce directions, dithering, area light samples).
    pub seed: u64,
}

impl Default for BakeOptions {
    fn default() -> Self {
        Self::from_settings(&BakeSettings::default(), DynamicGeometryMode::default(), 0)
    }
}

impl BakeOptions {
    pub fn from_settings(
        settings: &BakeSettings,
        dynamic_geometry: DynamicGeometryMode,
        seed: u64,
    ) -> Self {
        Self {
            pixel_density: settings.pixel_density,
            max_lightmap_size: settings.max_lightmap_size,
            bounce_compression: settings.bounce_compression,
            dynamic_geometry,
            skip_bounce: false,
            photon_cube_resolution: 512,
            area_light_samples: settings.area_light_samples,
            area_light_min_samples: settings.area_light_min_samples,
            sun_map_resolution: settings.sun_map_resolution,
            sky_samples: settings.sky_samples,
            sky_probe_spacing: settings.sky_probe_spacing,
            colored_bounce: settings.colored_bounce,
            seed,
        }
    }
}

/// Why a bake failed.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    /// The bake was cancelled through [`super::BakeProgress::cancel`].
    Cancelled,
    /// Two meshes or two lights share a [`BakeId`].
    DuplicateId { kind: &'static str, id: BakeId },
    /// A mesh's data is inconsistent (lengths, indices, non-finite positions).
    InvalidMesh { id: BakeId, reason: String },
    /// A light has a non-finite position or a radius that isn't positive.
    InvalidLight { id: BakeId, reason: String },
    /// The options are unusable.
    InvalidOptions(String),
}

impl fmt::Display for BakeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "the bake was cancelled"),
            Self::DuplicateId { kind, id } => {
                write!(f, "two {kind} share BakeId {:#018x}", id.0)
            }
            Self::InvalidMesh { id, reason } => write!(f, "mesh {:#018x}: {reason}", id.0),
            Self::InvalidLight { id, reason } => write!(f, "light {:#018x}: {reason}", id.0),
            Self::InvalidOptions(reason) => write!(f, "invalid bake options: {reason}"),
        }
    }
}

impl std::error::Error for BakeError {}

impl BakeMesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Checks lengths, index ranges and finiteness.
    pub fn validate(&self) -> Result<(), BakeError> {
        let invalid = |reason: String| BakeError::InvalidMesh {
            id: self.id,
            reason,
        };
        let vertices = self.positions.len();
        if !self.indices.len().is_multiple_of(3) {
            return Err(invalid(format!(
                "{} indices is not a multiple of 3",
                self.indices.len()
            )));
        }
        if let Some(index) = self.indices.iter().find(|&&i| i as usize >= vertices) {
            return Err(invalid(format!(
                "index {index} is out of range for {vertices} vertices"
            )));
        }
        if self.uv1.len() != vertices {
            return Err(invalid(format!(
                "{} UV1 coordinates for {vertices} vertices",
                self.uv1.len()
            )));
        }
        if !self.normals.is_empty() && self.normals.len() != vertices {
            return Err(invalid(format!(
                "{} normals for {vertices} vertices",
                self.normals.len()
            )));
        }
        if self.positions.iter().any(|p| !p.is_finite()) {
            return Err(invalid("non-finite vertex position".into()));
        }
        if let Some(uv0) = &self.uv0
            && uv0.len() != vertices
        {
            return Err(invalid(format!(
                "{} UV0 coordinates for {vertices} vertices",
                uv0.len()
            )));
        }
        if let Some(alpha) = &self.alpha {
            if self.uv0.is_none() {
                return Err(invalid("an alpha texture needs UV0".into()));
            }
            let texels = alpha.width as usize * alpha.height as usize;
            if alpha.rgba8.len() != texels * 4 {
                return Err(invalid("alpha texture size doesn't match its data".into()));
            }
        }
        Ok(())
    }
}

impl BakeLight {
    pub fn validate(&self) -> Result<(), BakeError> {
        let invalid = |reason: &str| BakeError::InvalidLight {
            id: self.id,
            reason: reason.into(),
        };
        if !self.position.is_finite() {
            return Err(invalid("non-finite position"));
        }
        if !self.shape.is_distant() && !(self.radius.is_finite() && self.radius > 0.0) {
            return Err(invalid("radius must be positive and finite"));
        }
        match self.shape {
            LightShape::Point => {}
            LightShape::Area(area) => {
                if !area.size.is_finite() || area.size.cmplt(Vec2::ZERO).any() {
                    return Err(invalid("area light size must be finite and not negative"));
                }
                if !area.rotation.is_finite() || !area.rotation.is_normalized() {
                    return Err(invalid("area light rotation must be a unit quaternion"));
                }
            }
            LightShape::Sun(sun) => {
                if !sun.rotation.is_finite() || !sun.rotation.is_normalized() {
                    return Err(invalid("sun rotation must be a unit quaternion"));
                }
                if !(0.0..=DynamicLight::MAX_ANGULAR_DIAMETER).contains(&sun.angular_diameter) {
                    return Err(invalid("sun angular diameter must be 0 to 10 degrees"));
                }
            }
            LightShape::Sky => {}
        }
        Ok(())
    }

    /// Whether the tracer bakes bounce lighting for this light. Skies never do; suns gather
    /// their bounce over each texel's hemisphere instead of from a photon cube.
    pub fn bakes_bounce(&self, options: &BakeOptions) -> bool {
        self.illumination == Illumination::SingleBounce
            && !options.skip_bounce
            && !matches!(self.shape, LightShape::Sky)
    }

    /// Whether shadow rays of this light alpha-test transparent surfaces.
    pub fn alpha_tested(&self) -> bool {
        self.transparency != Transparency::Disabled
    }
}

impl BakeOptions {
    pub fn validate(&self) -> Result<(), BakeError> {
        if !(self.pixel_density.is_finite() && self.pixel_density > 0.0) {
            return Err(BakeError::InvalidOptions(
                "pixel density must be positive".into(),
            ));
        }
        if self.max_lightmap_size < 2 {
            return Err(BakeError::InvalidOptions(
                "the maximum shadow map size must be at least 2".into(),
            ));
        }
        if self.photon_cube_resolution == 0 {
            return Err(BakeError::InvalidOptions(
                "the photon cube resolution must be positive".into(),
            ));
        }
        if self.area_light_samples == 0 {
            return Err(BakeError::InvalidOptions(
                "area lights need at least one sample".into(),
            ));
        }
        if !(1..=MAX_SUN_MAP_RESOLUTION).contains(&self.sun_map_resolution) {
            return Err(BakeError::InvalidOptions(format!(
                "the sun map resolution must be 1 to {MAX_SUN_MAP_RESOLUTION}"
            )));
        }
        if self.sky_samples == 0 {
            return Err(BakeError::InvalidOptions(
                "skies need at least one sample".into(),
            ));
        }
        if !(self.sky_probe_spacing.is_finite() && self.sky_probe_spacing > 0.0) {
            return Err(BakeError::InvalidOptions(
                "the sky probe spacing must be positive".into(),
            ));
        }
        Ok(())
    }
}
