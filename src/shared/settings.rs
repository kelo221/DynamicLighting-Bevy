//! Scene-wide settings read by several slices.

use bevy::prelude::*;

/// Scene-wide lighting settings, the counterpart of Henry's light manager settings.
#[derive(Resource, Reflect, Clone, Debug, PartialEq)]
#[reflect(Resource, Default, Debug, Clone, PartialEq)]
pub struct DynamicLightingSettings {
    /// Light added to every lit surface. Alpha is a strength: the color is scaled by alpha², which
    /// gives finer control near black. See [`Self::ambient_linear`].
    pub ambient: Color,
    /// Most realtime lights evaluated per frame. Realtime lights are lights marked realtime plus
    /// baked lights that moved; when there are more, the ones nearest the camera win.
    pub realtime_light_budget: u32,
    /// From this many realtime lights on, each fragment only visits the realtime lights that can
    /// reach its cell of a grid over the camera's view. With fewer, every fragment visits every
    /// realtime light, which costs about the same as finding the cell. 0 always uses the grid.
    pub realtime_cluster_min_lights: u32,
    /// Most lights with realtime shadow cubemaps at once.
    pub shadow_cubemap_budget: u32,
    /// Compiled into the lit shader: changing it recompiles the lit pipelines, which stalls lit
    /// meshes for a second or more.
    pub quality: RuntimeQuality,
    /// The lighting paths compiled into the lit shader; see [`ShaderFeatures`].
    pub shader_features: ShaderFeatures,
    pub dynamic_geometry: DynamicGeometryMode,
    pub tracking: TrackingMode,
    pub triangle_ids: TriangleIdSource,
    pub debug_view: DebugView,
    /// Seed for every random choice: effect randomness at runtime and sampling during bakes.
    pub seed: u64,
}

impl Default for DynamicLightingSettings {
    fn default() -> Self {
        Self {
            ambient: Color::srgba(1.0, 1.0, 1.0, 32.0 / 255.0),
            realtime_light_budget: 32,
            realtime_cluster_min_lights: 8,
            shadow_cubemap_budget: 16,
            quality: RuntimeQuality::default(),
            shader_features: ShaderFeatures::ALL,
            dynamic_geometry: DynamicGeometryMode::default(),
            tracking: TrackingMode::default(),
            triangle_ids: TriangleIdSource::default(),
            debug_view: DebugView::None,
            seed: 0,
        }
    }
}

impl DynamicLightingSettings {
    /// The ambient color as the shaders use it: linear RGB scaled by alpha².
    pub fn ambient_linear(&self) -> [f32; 3] {
        let color = self.ambient.to_linear();
        let strength = color.alpha * color.alpha;
        [
            color.red * strength,
            color.green * strength,
            color.blue * strength,
        ]
    }
}

/// The lighting paths compiled into the lit shader, all on by default. A path a scene never uses
/// still costs every lit pixel, because the shader reserves registers for it; switching it off
/// works like leaving out one of Henry's shader keywords. A light or mesh that needs a path that
/// is off renders without it, and a warning names the path. Changing these recompiles the lit
/// pipelines, which stalls lit meshes for a second or more, so set them up front.
#[derive(Reflect, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub struct ShaderFeatures {
    /// Spot and discoball cones, cookies and the pattern types (wave, interference, rotor, shock,
    /// disco). Off: these lights shine like point lights.
    pub light_patterns: bool,
    /// Rectangle and tube lights. Off: they shine like point lights at their position.
    pub area_lights: bool,
    /// Realtime lights, which include baked lights that moved or grew. Off: they add no light.
    pub realtime_lights: bool,
    /// Realtime shadow cubemaps. Off: lights that have one are unshadowed on moving meshes.
    pub realtime_shadows: bool,
    /// Meshes without bake data, such as moving meshes. Off: they only get ambient light.
    pub unbaked_meshes: bool,
    /// Baked bounce light. Off: baked lights add only their direct light.
    pub bounce: bool,
}

impl ShaderFeatures {
    pub const ALL: Self = Self {
        light_patterns: true,
        area_lights: true,
        realtime_lights: true,
        realtime_shadows: true,
        unbaked_meshes: true,
        bounce: true,
    };
    pub const NONE: Self = Self {
        light_patterns: false,
        area_lights: false,
        realtime_lights: false,
        realtime_shadows: false,
        unbaked_meshes: false,
        bounce: false,
    };

    /// The number of features in [`Self::list`].
    pub const COUNT: usize = 6;

    /// `(name, enabled)` of every feature, for messages and the variant module.
    pub fn list(&self) -> [(&'static str, bool); Self::COUNT] {
        [
            ("light_patterns", self.light_patterns),
            ("area_lights", self.area_lights),
            ("realtime_lights", self.realtime_lights),
            ("realtime_shadows", self.realtime_shadows),
            ("unbaked_meshes", self.unbaked_meshes),
            ("bounce", self.bounce),
        ]
    }
}

impl Default for ShaderFeatures {
    fn default() -> Self {
        Self::ALL
    }
}

/// Shadow filtering quality. Meant for an in-game graphics setting.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum RuntimeQuality {
    /// Cheap bilinear shadow filtering; no realtime shadows and no bounce lighting.
    IntegratedGraphics,
    /// 3×3 percentage-closer filtering.
    Low,
    /// Bilinear filtering.
    #[default]
    Medium,
    /// Bilinear filtering plus a 5×5 Gaussian.
    High,
}

impl RuntimeQuality {
    /// The level shaders compare against: 0 for integrated graphics up to 3 for high.
    pub const fn level(self) -> u32 {
        match self {
            Self::IntegratedGraphics => 0,
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
        }
    }
}

/// How meshes without baked data (moving or unbaked meshes) are shadowed by baked lights. In
/// both cube modes, rectangle and tube lights cast soft shadows sized by the blocker distance.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum DynamicGeometryMode {
    /// Lit by every light in range, without shadows; light leaks through walls.
    LightingOnly,
    /// Low-resolution baked distance cubes with cubic (voxel-like) interpolation.
    DistanceCubes,
    /// Baked distance cubes with angular filtering: soft penumbras that widen with distance.
    #[default]
    Angular,
}

/// How often baked light positions are re-read from their transforms (Henry's setting).
///
/// Both modes give the same result here: Bevy's transforms are change-tracked and cheap to read,
/// so light positions are always current. The setting is kept so scenes that set it still load.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum TrackingMode {
    /// Every frame.
    #[default]
    Live,
    /// Only when a light's transform changes or it is added. Rotation is always tracked.
    Relaxed,
}

/// How lit fragments learn which baked triangle they belong to, which baked shadows are keyed on.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum TriangleIdSource {
    /// `@builtin(primitive_index)` where the GPU supports it (desktop Vulkan, DX12 and Metal),
    /// otherwise [`Self::VertexAttribute`].
    #[default]
    Auto,
    /// A per-triangle vertex attribute. Baked meshes are de-indexed when they load, which
    /// triples the vertex count of smooth meshes. Works on every GPU; set it to try the fallback
    /// on a GPU that has primitive indices.
    VertexAttribute,
}

/// Replaces the shading of every `DynLitMaterial` to show what the lighting does.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[reflect(Default, Debug, PartialEq, Hash, Clone)]
pub enum DebugView {
    /// Normal shading.
    #[default]
    None,
    /// White surfaces without textures or emission: only light, shadows and ambient show.
    Lighting,
    /// How many lights reach each fragment: black for none, then blue, green, yellow and red
    /// at [`DEBUG_LIGHT_COUNT_MAX`] or more.
    LightCount,
}

impl DebugView {
    pub const ALL: [Self; 3] = [Self::None, Self::Lighting, Self::LightCount];
}

/// The light count shown as full red by [`DebugView::LightCount`].
pub const DEBUG_LIGHT_COUNT_MAX: u32 = 16;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_henrys_manager() {
        let settings = DynamicLightingSettings::default();
        assert_eq!(settings.realtime_light_budget, 32);
        assert_eq!(settings.shadow_cubemap_budget, 16);
        assert_eq!(settings.quality, RuntimeQuality::Medium);
        assert_eq!(settings.dynamic_geometry, DynamicGeometryMode::Angular);
        assert_eq!(settings.tracking, TrackingMode::Live);
        assert_eq!(settings.triangle_ids, TriangleIdSource::Auto);
    }

    #[test]
    fn ambient_is_scaled_by_alpha_squared() {
        let settings = DynamicLightingSettings {
            ambient: Color::linear_rgba(0.5, 1.0, 0.25, 0.5),
            ..Default::default()
        };
        assert_eq!(settings.ambient_linear(), [0.125, 0.25, 0.0625]);

        let [r, g, b] = DynamicLightingSettings::default().ambient_linear();
        let expected = (32.0_f32 / 255.0).powi(2);
        assert!((r - expected).abs() < 1e-6 && r == g && g == b);
    }
}
