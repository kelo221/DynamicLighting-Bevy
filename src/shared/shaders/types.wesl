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

// The light buffer header. Mirrors `dynlight::shared::GpuLightGlobals` (32 bytes).
struct LightGlobals {
    // Linear RGB, already scaled by alpha².
    ambient: vec3<f32>,
    // Seconds since startup.
    time: f32,
    baked_count: u32,
    realtime_count: u32,
    // 0 integrated graphics, 1 low, 2 medium, 3 high.
    quality: u32,
    // `GLOBALS_FLAG_*` bits.
    flags: u32,
}

// Dynamic geometry ignores the baked distance cubes.
const GLOBALS_FLAG_LIGHTING_ONLY: u32 = 1u;
// Debug view: white surfaces without textures or emission.
const GLOBALS_FLAG_DEBUG_LIGHTING: u32 = 2u;
// Debug view: a heat map of the lights reaching each fragment.
const GLOBALS_FLAG_DEBUG_LIGHT_COUNT: u32 = 4u;

// The whole light buffer: the header, then the baked slots, then the realtime lights.
struct DynamicLights {
    globals: LightGlobals,
    lights: array<GpuLight>,
}

const LIGHT_FLAG_REALTIME: u32 = 32u;
const LIGHT_TYPE_MASK: u32 = 960u;
const LIGHT_SHIMMER_MASK: u32 = 3072u;
const LIGHT_FLAG_SHADOW_CUBEMAP: u32 = 32768u;
const LIGHT_FLAG_COOKIE: u32 = 65536u;
const LIGHT_BOUNCE_MASK: u32 = 917504u;

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

// Bounce data bits per pixel, 1 to 8.
fn light_bounce_bits_per_pixel(light: GpuLight) -> u32 {
    return 1u + ((light.channel & LIGHT_BOUNCE_MASK) >> 17u);
}
