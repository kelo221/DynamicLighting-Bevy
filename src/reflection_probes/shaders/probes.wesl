// Reflection probes: whether a probe's box holds a point, and the parallax-corrected direction
// to sample its cube in. Mirrors `reflection_probes::records`. The caller reads the records
// (`DynamicLights::probes`, smallest box first) and passes in the rows of a probe's
// world-to-box transform, so only those are loaded while it looks for the probe.

// No probe holds the point: surfaces reflect the sky.
const NO_PROBE: u32 = 0xffffffffu;

// `world` in a probe's box space, where the box is the ±1 cube.
fn probe_box_point(x: vec4<f32>, y: vec4<f32>, z: vec4<f32>, world: vec3<f32>) -> vec3<f32> {
    let p = vec4(world, 1.0);
    return vec3(dot(x, p), dot(y, p), dot(z, p));
}

// How far past its sides, as a share of its half extents, a box still holds points, so walls
// that lie on its sides use it everywhere; `HOLD_MARGIN` in `records`.
const HOLD_MARGIN: f32 = 0.01;

fn probe_holds(x: vec4<f32>, y: vec4<f32>, z: vec4<f32>, world: vec3<f32>) -> bool {
    return all(abs(probe_box_point(x, y, z, world)) <= vec3(1.0 + HOLD_MARGIN));
}

// The direction to sample a probe's cube in for a ray from `world` (inside the box) along
// `direction`: from the capture point `capture` to where the ray leaves the box.
fn probe_box_projected(x: vec4<f32>, y: vec4<f32>, z: vec4<f32>, capture: vec3<f32>, world: vec3<f32>, direction: vec3<f32>) -> vec3<f32> {
    let start = probe_box_point(x, y, z, world);
    let step = vec3(dot(x.xyz, direction), dot(y.xyz, direction), dot(z.xyz, direction));
    // Per axis, the distance to the face the ray moves toward; axes it doesn't move along
    // never end it.
    let moving = abs(step) >= vec3(1.0e-8);
    let safe = select(vec3(1.0), step, moving);
    let exits = select(vec3(3.0e38), (sign(safe) - start) / safe, moving);
    let t = max(min(exits.x, min(exits.y, exits.z)), 0.0);
    return world + direction * t - capture;
}
