// The GPU light record and its channel flags. Mirrors `dynlight::shared::GpuLight` in Rust
// (112 bytes); the layout test in `src/shared/gpu_light.rs` compares the two.

struct GpuLight {
    position: vec3<f32>,
    // Negative disables the light in every shader path.
    radius_sqr: f32,
    channel: u32,
    intensity: f32,
    gp1: f32,
    gp2: f32,
    color: vec3<f32>,
    gp3: f32,
    up: vec3<f32>,
    shimmer_scale: f32,
    forward: vec3<f32>,
    shimmer_modifier: f32,
    volumetric_intensity: f32,
    volumetric_visibility: f32,
    cookie_index: u32,
    shadow_cubemap_index: u32,
    bounce_color: vec3<f32>,
    falloff: f32,
}

// The light buffer header. Mirrors `dynlight::shared::GpuLightGlobals` (48 bytes).
struct LightGlobals {
    // Linear RGB, already scaled by alpha².
    ambient: vec3<f32>,
    // Seconds since startup.
    time: f32,
    baked_count: u32,
    realtime_count: u32,
    // 0 integrated graphics, 1 low, 2 medium, 3 high. Shaders use the variant's `QUALITY`.
    quality: u32,
    // `GLOBALS_FLAG_*` bits.
    flags: u32,
    // Distant lights (suns) among the baked slots: the last `baked_distant` of them. The light
    // BVH holds only the slots before them.
    baked_distant: u32,
    // Distant lights among the realtime lights: the first `realtime_distant` of them. The
    // realtime light grid holds only the lights after them.
    realtime_distant: u32,
    _padding: vec2<u32>,
}

// Dynamic geometry ignores the baked distance cubes.
const GLOBALS_FLAG_LIGHTING_ONLY: u32 = 1u;
// Debug view: white surfaces without textures or emission.
const GLOBALS_FLAG_DEBUG_LIGHTING: u32 = 2u;
// Debug view: a heat map of the lights reaching each fragment.
const GLOBALS_FLAG_DEBUG_LIGHT_COUNT: u32 = 4u;

// The sky's irradiance over π in the sky's frame, as spherical harmonics of bands 0 to 2
// (`sh[i].xyz` per channel). Mirrors `dynlight::shared::GpuSkyIrradiance` (144 bytes).
struct SkyIrradiance {
    sh: array<vec4<f32>, 9>,
}

// One cascade of the sun's realtime shadows: a world point's texture coordinates and depth are
// `dot(row, vec4(world, 1.0))` for the rows `u`, `v` and `depth` (0 at the sunward end, 1 at the
// far end). Mirrors `dynlight::shared::GpuSunCascade` (64 bytes).
struct SunCascade {
    u: vec4<f32>,
    v: vec4<f32>,
    depth: vec4<f32>,
    // World size of one texel.
    texel_size: f32,
    // In depth units.
    depth_bias: f32,
    _padding: vec2<f32>,
}

// The cascades of the one sun with realtime shadows, nearest the camera first; `count` is 0
// without one. Mirrors `dynlight::shared::GpuSunShadow` (208 bytes).
struct SunShadow {
    cascades: array<SunCascade, 3>,
    count: u32,
    // `SUN_SHADOW_*` bits.
    flags: u32,
    _padding: vec2<u32>,
}

// The cascades hold only meshes without bake data, so baked receivers aren't in them.
const SUN_SHADOW_MOVING_ONLY: u32 = 1u;

// A reflection probe: a world point `p` lies in its box when each of `dot(row, vec4(p, 1.0))`
// for the rows `x`, `y` and `z` is within ±1. Mirrors `dynlight::shared::GpuReflectionProbe`
// (208 bytes).
struct ReflectionProbe {
    x: vec4<f32>,
    y: vec4<f32>,
    z: vec4<f32>,
    // Where its cube was captured, in world space.
    capture: vec3<f32>,
    // Its cube in the reflection cube array.
    layer: u32,
    // Irradiance over π at the capture point, in world space.
    irradiance: SkyIrradiance,
}

// The reflection probes, smallest box first. Mirrors `dynlight::shared::GpuReflectionProbes`.
struct ReflectionProbes {
    count: u32,
    _padding: array<u32, 3>,
    probes: array<ReflectionProbe, 16>,
}

// The whole light buffer: the header, the sky, the sun's cascades, the reflection probes, then
// the baked slots and the realtime lights.
struct DynamicLights {
    globals: LightGlobals,
    sky: SkyIrradiance,
    sun_shadow: SunShadow,
    probes: ReflectionProbes,
    lights: array<GpuLight>,
}

const LIGHT_FLAG_REALTIME: u32 = 32u;
const LIGHT_TYPE_MASK: u32 = 960u;
const LIGHT_SHIMMER_MASK: u32 = 3072u;
const LIGHT_FLAG_SHADOW_CUBEMAP: u32 = 32768u;
const LIGHT_FLAG_COOKIE: u32 = 65536u;
const LIGHT_BOUNCE_MASK: u32 = 917504u;
const LIGHT_SHADOW_MASK: u32 = 7340032u;
const LIGHT_FLAG_COLORED_BOUNCE: u32 = 8388608u;

const LIGHT_TYPE_POINT: u32 = 0u;
const LIGHT_TYPE_SPOT: u32 = 64u;
const LIGHT_TYPE_DISCOBALL: u32 = 128u;
const LIGHT_TYPE_WAVE: u32 = 192u;
const LIGHT_TYPE_INTERFERENCE: u32 = 256u;
const LIGHT_TYPE_ROTOR: u32 = 320u;
const LIGHT_TYPE_SHOCK: u32 = 384u;
const LIGHT_TYPE_DISCO: u32 = 448u;
const LIGHT_TYPE_RECTANGLE: u32 = 512u;
const LIGHT_TYPE_TUBE: u32 = 576u;
const LIGHT_TYPE_SUN: u32 = 640u;
const LIGHT_TYPE_SKY: u32 = 704u;

const LIGHT_SHIMMER_NONE: u32 = 0u;
const LIGHT_SHIMMER_WATER: u32 = 1024u;
const LIGHT_SHIMMER_RANDOM: u32 = 2048u;

fn light_is_realtime(light: GpuLight) -> bool {
    return (light.channel & LIGHT_FLAG_REALTIME) != 0u;
}

// One of the `LIGHT_TYPE_*` constants.
fn light_type(light: GpuLight) -> u32 {
    return light.channel & LIGHT_TYPE_MASK;
}

// Rectangle and tube lights, whose light comes from a shape with half extents (gp1, gp2).
fn light_is_area(light: GpuLight) -> bool {
    let kind = light_type(light);
    return kind == LIGHT_TYPE_RECTANGLE || kind == LIGHT_TYPE_TUBE;
}

// Light from infinitely far away, without a position or radius: suns and skies. `forward`
// points towards a sun; `gp1` and `gp2` are the cosine and sine of its disk's angular radius. A
// sky's `forward` and `up` turn its image.
fn light_is_distant(light: GpuLight) -> bool {
    let kind = light_type(light);
    return kind == LIGHT_TYPE_SUN || kind == LIGHT_TYPE_SKY;
}

// How far an area light's shape reaches from its position; 0 for the other types. An area
// light's `radius_sqr` is its reach past the shape, so `sqrt(radius_sqr) + light_extent(light)`
// is the light's radius around its position.
fn light_extent(light: GpuLight) -> f32 {
    if light_is_area(light) {
        return length(vec2(light.gp1, light.gp2));
    }
    return 0.0;
}

// One of the `LIGHT_SHIMMER_*` constants.
fn light_shimmer(light: GpuLight) -> u32 {
    return light.channel & LIGHT_SHIMMER_MASK;
}

fn light_has_shadow_cubemap(light: GpuLight) -> bool {
    return (light.channel & LIGHT_FLAG_SHADOW_CUBEMAP) != 0u;
}

fn light_has_cookie(light: GpuLight) -> bool {
    return (light.channel & LIGHT_FLAG_COOKIE) != 0u;
}

// Whether the baked bounce texels hold red, green and blue.
fn light_has_colored_bounce(light: GpuLight) -> bool {
    return (light.channel & LIGHT_FLAG_COLORED_BOUNCE) != 0u;
}

// Bounce data bits per pixel (per channel when colored), 1 to 8.
fn light_bounce_bits_per_pixel(light: GpuLight) -> u32 {
    return 1u + ((light.channel & LIGHT_BOUNCE_MASK) >> 17u);
}

// Baked shadow bits per texel, 1 to 8: 1 for Henry's hard shadows, more for the soft
// visibility of area lights.
fn light_shadow_bits_per_texel(light: GpuLight) -> u32 {
    return 1u + ((light.channel & LIGHT_SHADOW_MASK) >> 20u);
}
