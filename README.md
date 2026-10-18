# dynlight

Baked per-texel shadows with live light evaluation for [Bevy](https://bevy.org) 0.20: a port of Henry de Jongh's [Dynamic Lighting](https://github.com/Henry00IS/DynamicLighting) for Unity.

A CPU raytracer bakes, for every static triangle, which lights reach it and a shadow mask per texel (1 bit, 4 bits of soft visibility for area lights and the sun, or 8 bits of how much of the sky a texel sees). At runtime a custom material evaluates each of those lights live (color, intensity, effects, light types, cookies, shimmer) and multiplies by the baked shadow bits, so baked lights can still flicker, pulse and change color. Moving objects, realtime lights, bounce lighting, realtime shadows and volumetric fog are layered on top.

The crate is its own lighting system. It never uses Bevy's lights (`PointLight`, `SpotLight`, `DirectionalLight`, ambient light, environment maps, skyboxes, shadow maps) or `bevy_pbr`'s lighting shaders.

## Features

- **Lights:** Henry's 8 types (point, spot, discoball, wave, interference, rotor, shock, disco) plus rectangle and tube area lights (shaded from their shape, with soft baked shadows; rectangles reflect through linearly transformed cosines) and a sun (parallel light from a disk `angular_diameter` degrees wide, with soft baked shadows), 15 intensity effects, water and firelight shimmer, spot cookies. Presets: `DynamicLight::spot()`, `discoball()`, `rectangle(w, h)`, `tube(length)`, `sun()`, and `sun_light(direction)`, a sun shining along a direction.
- **Sky:** an HDR equirect image (`SkyEnvironment`) lights every surface from all around and is drawn behind the scene. It is prefiltered on the CPU when it loads: nine spherical-harmonic coefficients for diffuse light and a GGX cube for Metallic reflections. Its color, intensity and effects stay live, and turning its entity turns it. Its baked shadows store how much of the sky each texel sees, so it no longer reaches under roofs; turning the sky or swapping its image keeps the bake.
- **Baked shadows:** per-texel shadow bits for every (triangle, light) pair, with seam padding, optional alpha-tested transparency, and four filtering quality tiers.
- **Bounce light:** one baked bounce per light, stored per texel at 4 to 8 bits.
- **Dynamic geometry:** moving meshes find the lights around them through a baked light BVH and are shadowed by baked distance cubes, with angular or cubic filtering. A baked sun shadows them through its sun map, a depth map of the static scene seen from the sun, and a baked sky through a grid of sky probes.
- **Realtime lights and shadows:** lights marked `realtime` are evaluated every frame, nearest first within a budget, and each pixel only visits the realtime lights that reach its cell of a view-space grid. `ShadowMode::Realtime` adds a variance shadow cubemap so moving objects cast shadows; on a sun it adds three shadow cascades over the camera's view instead. A cubemap only redraws the faces where something changed, and a cascade only when the camera moves past its snapping step or a caster in it changes. A light or sun at its baked position leaves baked meshes out, so a still scene renders no shadow faces or cascades at all. Area lights blur their cubemaps wider.
- **Materials:** `DynLitMaterial` covers Henry's Diffuse, Metallic (GGX) and Unlit shaders, with normal, occlusion and metallic-roughness maps. glTF `StandardMaterial`s and light extras are converted on load.
- **Volumetric fog:** sphere, box, cone and square-shaft (`Frustum`) fog volumes per light, as a post-process pass. The shaft starts at an area light's size, for fluorescent fixtures.
- **Debug views:** `DynamicLightingSettings::debug_view` shows lighting on white surfaces, or a heat map of how many lights reach each fragment.

## Compared with Henry's original

The core is Henry's design, ported closely: baking per-triangle light lists with a 1-bit shadow mask per texel, his ±2-texel padding and shadow filters, bounce light, distance cubes for moving objects, the eight light types and fifteen effects, his attenuation and his volumetric fog. The baked triangle data keeps his layout. What differs:

**New in dynlight**

- **Area lights.** Rectangle and tube lights are shaded from their shape and bake soft shadows: each texel stores how much of the shape it sees in 4 bits (16 levels) instead of Henry's single shadow bit, so penumbras are smooth and harden where objects touch the ground. Moving objects get soft shadows from the light's distance cube, with penumbras that widen with the distance to the blocker. Otherwise they follow his rules: one light at a fixed position with a `radius`, and moving, turning or resizing a baked area light falls back to realtime lighting until the next bake.
- **A sun.** `LightType::Sun` is a light at infinity with a disk (0.53° by default, like the real sun). Its baked shadows are Henry's single bit per texel for a disk of 0°, and 4 bits of soft visibility otherwise, so penumbras widen with the distance to the blocker. Moving objects get its shadows from a baked sun map. It keeps Henry's model of a light: color, intensity and effects change live, and turning a baked sun falls back to realtime lighting until the next bake. With `ShadowMode::Realtime`, moving objects cast sun shadows too, through three variance shadow cascades up to `sun_shadow_distance` (50 m) that hold only the moving meshes while the sun stands where it was baked. Suns bake no bounce light.
- **An HDRI sky.** Henry's scenes get a flat ambient color; `LightType::Sky` lights them from an HDR image, with reflections on Metallic materials, and draws the image as the background. It is a light like the others (color, intensity and effects change live). Its baked shadow is how much of the sky each texel sees, in 8 bits from 64 rays over the hemisphere, and it dims the sky's reflection as much as its diffuse light, so the sky stays out from under roofs. It doesn't depend on the image, so the sky can turn or change its image without a new bake. Moving objects get sky shadows from a baked grid of probes about a meter apart. Faces turned to the ground see no sky and stay dark, since nothing bakes bounce light from the sun or the sky.
- **Cached realtime shadows.** Shadow cubemaps redraw only the faces where something changed, and leave baked meshes out while their light stands where it was baked. Area lights blur their cubemaps wider.
- **Clustered realtime lights.** Henry's shader visits every realtime light at every pixel; dynlight lists the realtime lights per cell of a view-space grid, so with 1000 realtime lights our lit pass takes 2.0 ms of GPU time, where Henry's whole frame takes 6.1 ms (see Performance).
- **Fog:** a square shaft (`VolumetricShape::Frustum`) for panel lights, and box fog that turns with its light.
- **Platforms and tools:** bakes in memory and in the browser (WebGPU), a triangle-ID fallback for GPUs without primitive indices, debug views (lighting only, light count), and a CPU reference of the shader math that is checked against GPU readback.

**Different from Henry's**

- **Bevy instead of Unity.** Nothing uses Unity's lighting, random numbers, Perlin noise or camera rendering. Bake data is matched to entities by their `Name` path (`BakeId`) instead of Unity instance IDs.
- **Bugs fixed,** e.g. intensity applied twice in Metallic shading, seam padding wrapping across rows, a divide by zero for spot cones with equal cutoffs, bounce blur passes reading and writing the same texture, and realtime shadows switching off for good after a frame that used every shadow cubemap.
- **Smoother realtime shadows:** a 5-tap blur at σ 1.5 instead of 3 taps at σ 1, a smooth fade at the terminator instead of the `1 − NdotL` light-bleeding term, and 32-bit moments wherever the GPU can filter them.
- **Metallic shading** doesn't divide diffuse light by π, so a rough non-metal looks the same under Metallic and Diffuse shading.
- **Smaller changes:** a baked light counts as moved beyond 1 mm (Henry: 0.01 mm), shadow cubemaps end at the light's radius (Henry: 3× the radius), cubemaps go to the lights nearest the camera, transparent shadows alpha-test the texture directly instead of filtering photon cubes, and `TrackingMode` has no effect.

PLAN.md §12 lists every difference with its reason ("Quirks in Henry's code" and "Beyond Henry").

## Live demos

[Open the interactive WebGPU demos](https://kelo221.github.io/DynamicLighting-Bevy/): the lighting test scene, realtime shadows, the Feathers light inspector and a 256-light stress scene.

They need WebGPU: Chrome or Edge 113+, Safari 26+, or Firefox 141+ on Windows. WebGL2 can't run dynlight, because it reads storage buffers in fragment shaders.

- The lighting test scene and the inspector bake when they start, in the browser: the Preview preset at a lower texel density, so without bounce light. The bake blocks the page for a moment; the status line then shows how long it took.
- Click the canvas first for keyboard controls. In the lighting test scene, **WASD** or the arrow keys drive a cube (the key positions, so **WARS** on Colemak), **Shift** runs, **Space** switches its lantern, **B** rebakes and **V** cycles the debug views. Drag to orbit and scroll to zoom in the lighting test and stress scenes.
- The query string works like the command line: `?debug-view=light-count` starts the lighting test scene in a debug view, and `?lights=1024` changes the stress scene.

## Usage

```rust,no_run
use bevy::prelude::*;
use dynlight::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, DynamicLightingPlugin))
        .add_systems(Startup, setup)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<DynLitMaterial>>,
) {
    commands.spawn((
        Name::new("Floor"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(10.0, 10.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.8, 0.8, 0.8))),
        // Static: shadows on it are baked.
        BakeStatic,
    ));
    commands.spawn((
        Name::new("Lamp"),
        DynamicLight { radius: 6.0, ..default() },
        LightEffect::new(LightEffectKind::Candle),
        Transform::from_xyz(0.0, 2.0, 0.0),
    ));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 6.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}
```

Meshes need `DynLitMaterial`; Bevy's `StandardMaterial` is never lit by dynlight.

### Baking

1. Mark static meshes with `BakeStatic`. Give meshes and lights a `Name`: bake data is matched to entities by their name path, so names keep matching across runs and scene edits. Use `BakeId` to set an ID yourself.
2. Send a `BakeRequest { output: Some("assets/level.dynlight".into()), preset: BakePreset::Full }` message on native, or use `output: None` for an in-memory bake on any platform. The bake runs on the async compute pool and ends with a `BakeFinished` message. Use `BakePreset::Preview` to skip bounce light, and `BakeSettings` to set the texel density.
3. On success, put `report.handle` from `BakeFinished.result` into `ActiveBake.0`. Every successful bake is available immediately in `Assets<DynLightBake>`, whether or not a file was requested. Saved `.dynlight` files can also be loaded later through the asset server. Browser file outputs are rejected with a clear error; use `None` there.

Shadow maps use the mesh's UV1, or UV0 when there is no UV1; overlapping UVs are fine for shadows. Bounce light needs non-overlapping UV1, because it is one map per mesh. Meshes that change after the bake are detected through a content hash: they lose their baked shadows, with a warning, until the next bake. Lights that move fall back to realtime lighting until the next bake. Turning a light keeps its bake, except for area lights and suns: turning or resizing an area light, or turning a sun or changing its disk, has the same effect. A sun can move freely, because it has no position, and a sky can also turn and change its image.

### Sun and sky

```rust,ignore
// A sun with a soft disk, shining down at a slant.
commands.spawn((Name::new("Sun"), sun_light(Dir3::new(Vec3::new(-0.4, -1.0, -0.3)).unwrap())));
// The sky: an equirect image, turned about +Y with its entity.
commands.spawn((
    Name::new("Sky"),
    SkyEnvironment {
        // The sun light stands for the image's sun, so leave it out of the sky's light.
        max_radiance: Some(3.0),
        ..SkyEnvironment::new(asset_server.load("sky.exr"))
    },
    Transform::from_rotation(Quat::from_rotation_y(0.5)),
));
```

The image is equirectangular, in Blender's mapping: its center looks along +X and its top is straight up. Loading `.exr` or `.hdr` files needs Bevy's `exr` or `hdr` feature; 8-bit images work too. The sky is built on a background task once the image loads (25 ms for a 1024 × 512 image and 51 ms for 2048 × 1024 on a desktop CPU); `environment::SkyStatus::building` is true until it is ready. `max_radiance` limits the image's light but not the background, so a sun in the image isn't counted twice next to a sun light. A sky is baked unless it is `realtime`: each texel traces `BakeSettings::sky_samples` rays (64; at most 16 in Preview bakes), and moving objects read sky probes `BakeSettings::sky_probe_spacing` apart (1 m). A realtime sky casts no shadows. Give the sun `shadows: ShadowMode::Realtime` for moving objects to cast sun shadows; one sun has them at a time. Put `NoSkyBackground` on a camera to keep its clear color. One sky lights the scene at a time. A `DynamicLight::sky()` without `SkyEnvironment` is a uniform sky of its color, Henry's ambient light as a light with effects, and draws no background.

### Shader variants

The lit shader is compiled for `DynamicLightingSettings::quality` and for the lighting paths in `DynamicLightingSettings::shader_features`: light patterns and cookies, area lights, the sun, the sky, realtime lights, realtime shadows, meshes without bake data, and baked bounce light. Everything is on by default. A game that doesn't use some of them can switch them off for a faster shader:

```rust,ignore
app.insert_resource(DynamicLightingSettings {
    shader_features: ShaderFeatures { area_lights: false, realtime_shadows: false, ..ShaderFeatures::ALL },
    ..default()
});
```

Changing either setting at runtime recompiles the lit shaders, which takes a second or more. If the scene uses a path that is switched off for 3 seconds, a warning names it.

Or let dynlight pick the paths the scene uses, the way Henry's package sets its shader keywords when a scene loads:

```rust,ignore
app.insert_resource(DynamicLightingSettings {
    feature_selection: FeatureSelection::Automatic,
    ..default()
});
```

The lit shader then waits until the scene's bake is applied and compiles once with only the paths in use (at most `shader_features`). A path first needed later, such as the first spot light spawned in a scene of point lights, is added then, and the recompile hides lit meshes for a second or more. Applying another bake chooses again. It suits scenes whose kinds of lights and meshes are known when they load.

### Platforms

- **Primitive indices:** baked shadows are looked up by triangle. On GPUs with wgpu's `PRIMITIVE_INDEX` feature (desktop Vulkan, DX12, Metal) this is free. Elsewhere, baked meshes are de-indexed when they load and carry a triangle-ID vertex attribute instead. `DynamicLightingSettings::triangle_ids` forces either path.
- **Subgroups:** where wgpu has `SUBGROUP` (desktop Vulkan, DX12 with DXC, Metal), neighbouring pixels on one baked triangle or in one cell of realtime lights load each light once per subgroup. The image is the same either way; `DynamicLightingSettings::subgroups` can switch it off.
- **Shadow cubemaps:** use `Rg32Float` where the GPU can filter 32-bit floats, and `Rg16Float` otherwise.
- **Web:** WebGPU works, through the triangle-ID path (wgpu doesn't expose primitive indices on the web). Bakes run on the browser's main thread with `BakeRequest { output: None, .. }`. WebGL2 is not supported, because it has no storage buffers in fragment shaders.

## Examples

| Example | What it shows |
|---|---|
| `cargo run --example lighting_test` | Every light type, effect, shimmer, a cookie, fog volumes and bounce light, a fluorescent ceiling panel with a fog shaft and soft baked shadows, a wall tube, and a cube to drive around with a realtime-shadowed lantern. `--bake` (or B) bakes the scene; `--triangle-ids` uses the fallback path. |
| `cargo run --example realtime_shadows` | Orbiting point and spot lights with realtime shadow cubemaps over moving crates; logs the cube faces rendered per frame. `--still` stops everything, so no face renders. |
| `cargo run --release --example stress_lights -- --bake` | 1000 baked lights among about 1000 pillars; `--realtime` makes them all realtime (and drifting) instead. |
| `cargo run --example outdoor -- --bake` | A courtyard under a baked soft sun and a baked procedural sky, with a porch, a pergola, trees, a lamp post and two metal balls reflecting the sky; a rolling ball and a spinning crate take the sun's shadows from the sun map and the sky's from the sky probes, and cast theirs through the sun's realtime cascades. Later runs load the bake. `--hdri path.exr` lights it with an HDR image instead, turned so its sun matches the sun light. |
| `cargo run --example gpu_check` | Reads the rendered image back and compares it with the CPU reference of the light shaders. Exits with an error on a mismatch. |
| `cargo run --example inspector --features inspector` | A Bevy Feathers panel for editing one light live and baking with a button. |

All examples except `gpu_check` accept `--frames N` (exit after N frames) and `--screenshot path.png`.

The GPU checks need a GPU, so they are skipped by `cargo test`; run them with `cargo test --test gpu_examples -- --ignored --test-threads=1`.

Visual baselines guard the look: `cargo test --test gpu_baselines -- --ignored --test-threads=1` renders still frames of `lighting_test`, `outdoor`, `realtime_shadows` and `stress_lights` (`--capture path.png` freezes time and waits for the bake and the shaders) and compares them with `tests/baselines/` using NVIDIA's [FLIP](https://github.com/NVlabs/flip) metric. A failure leaves the new frame and an error map in `target/tmp/baselines/`. After an intended change to the look, rerun with `DYNLIGHT_BLESS=1` and review the new images before committing them.

## Performance

dynlight against Bevy's own lights and against Henry's package in Unity, in the same scene: the `stress_lights` layout, about 1000 pillars in an 80 m hall. Both harnesses live outside this repository: one spawns Bevy's `PointLight`s, the other is a Unity project. Unity builds each scene from the harness's scene description and bakes it with Henry's own baker.

Setup: 1920×1080, 4× MSAA, RTX 4080 SUPER, Direct3D 12 for all three, 512² shadow cubemaps. dynlight and Bevy use the release profile; Unity 6000.6.4f1 uses an IL2CPP release player, the Built-in pipeline and Henry's package at e2a51ad. Each figure is the median of 10 alternating runs (2026-10-17); the dynlight and Bevy cells were measured again later that day, after a round of performance work.

| Scene | Frame: dynlight | Frame: Bevy | Frame: Henry | GPU: dynlight | GPU: Bevy | GPU: Henry |
|---|---|---|---|---|---|---|
| 1000 still lights (dynlight and Henry: baked shadows; Bevy: no shadows) | 1.90 ms | 2.02 ms | 0.76 ms | 0.61 ms | 1.33 ms | 0.46 ms |
| 100 moving realtime lights, no shadows | 1.90 ms | 2.02 ms | 0.40 ms | 0.35 ms | 0.25 ms | 0.26 ms |
| 1000 moving realtime lights, no shadows | 1.94 ms | 2.18 ms | 6.28 ms | 1.45 ms | 1.33 ms | 6.09 ms |
| 15 of 100 lights with realtime shadows, scene still | 1.88 ms | 22.9 ms | 7.09 ms | 0.36 ms | – | 2.06 ms |
| 15 of 100 lights with realtime shadows, scene moving | 19.8 ms | 22.9 ms | 7.16 ms | – | – | 2.09 ms |

The GPU columns don't measure the same thing. dynlight's and Bevy's are the main lit pass alone (wgpu timestamps); the harness doesn't report it while shadows move or for Bevy's shadowed lights. Henry's is Unity's GPU time for the whole frame. The shadow rows use 15 lights because Henry's package turns realtime shadows off for good after a frame that fills all 16 of its cubemaps. Bevy doesn't sort opaque draws front to back, and its draw order changes from build to build, so the same scene can shade up to 30% more fragments in another build of either engine; the lit-pass numbers carry that uncertainty. A `DepthPrepass` on the camera makes each pixel shade once.

- **Frame time** in dynlight and Bevy is mostly engine overhead: about 1.9 ms for dynlight and 2.0–2.2 ms for Bevy's own lights. dynlight saves a little by not running Bevy's GPU light clustering, which it doesn't need. Unity's player needs about 0.4 ms.
- **Baked lights** shade faster than Bevy's lights without shadows, and their shadows cost nothing at runtime. Like Henry's package, dynlight compiles its lit shader for the quality tier and the material's shading model. Switching off the lighting paths this scene doesn't use (`shader_features`, or `FeatureSelection::Automatic`, which picks the same paths here; see Usage) brings the pass to 0.45 ms, level with Henry's 0.46 ms for the whole frame.
- **Realtime lights** are sorted into the cells of a view-space grid, as Bevy does, so a pixel only visits the lights that reach it. Henry's shader visits every realtime light at every pixel, so his 1000-light frame takes four times our lit pass.
- **Realtime shadows** are cached in dynlight: a cubemap face only redraws where something changed, so a still scene renders none. Bevy's and Henry's still scenes cost as much as their moving ones. When everything moves, all three redraw 90 faces per frame. Unity does that in about 7 ms; Bevy's per-view cost puts dynlight at 19.8 ms and Bevy at 22.9 ms.
- **The images match.** Rendered without tonemapping from the same camera, dynlight's and Henry's frames of the stress, light-type and shadow scenes differ on average by 0.01–0.14 of one 8-bit step. At most 0.2% of pixels differ by more than 16 steps, all along pillar edges.

Baking the 1000-light scene (1047 meshes, 6.3 M texels, 18 M rays) takes 1.3 s on a 32-thread desktop CPU; Henry's baker takes 22.2–22.6 s for the same scene in the Unity editor (direct light only for both, 16 texels per metre). `cargo bench --bench bake` measures the baker on its own (about 27 M shadow rays per second), and `cargo bench --bench clusters` the per-frame light grid (0.27 ms for 1000 lights).

## Development

Contributor rules (slice layout, 400-line file limit, what may and may not be ported) are in [`AGENTS.md`](AGENTS.md); the design and milestones are in [`PLAN.md`](PLAN.md). The `dev` feature links Bevy dynamically for faster rebuilds; the aliases in `.cargo/config.toml` enable it: `cargo check-dev`, `cargo test-dev`, `cargo run-dev --example lighting_test`, `cargo clippy-dev`.

### Building the web demos

Install the `wasm32-unknown-unknown` target, the `wasm-bindgen-cli` version that matches `wasm-bindgen` in `Cargo.lock` (the build script checks it) and, optionally, Binaryen's `wasm-opt`. Then run `bash web/build.sh` and serve `dist/`, e.g. `python -m http.server 8080 -d dist`. The script uses the size-optimized `wasm-release` profile, never the `dev` feature. GitHub Pages builds and deploys the same site on every push to `master`.

## Credits and licence

The lighting design, formulas and shaders are Henry de Jongh's, ported from [Henry00IS/DynamicLighting](https://github.com/Henry00IS/DynamicLighting) under the MIT licence; his notice is in [`LICENSES/DynamicLighting-MIT.txt`](LICENSES/DynamicLighting-MIT.txt). The shadow blur kernel comes from Daniel Ilett's SmoShaders (MIT), notice in [`LICENSES/SmoShaders-MIT.txt`](LICENSES/SmoShaders-MIT.txt). Rectangle-light highlights use the fitted tables of Heitz, Dupuy, Hill and Neubelt, "Real-Time Polygonal-Light Shading with Linearly Transformed Cosines" (SIGGRAPH 2016), BSD-2, notice in [`LICENSES/LTC-BSD-2.txt`](LICENSES/LTC-BSD-2.txt). [`docs/PORT_MAP.md`](docs/PORT_MAP.md) lists where every ported file comes from. Nothing is taken from Unity's own code; where Henry relied on Unity (random numbers, Perlin noise, camera rendering), dynlight uses its own implementations.

Code reused from the user-owned [bevyout](https://github.com/kelo221/bevyout) `DynamicLightsPort` branch is relicensed by its sole author under this crate's MIT OR Apache-2.0 terms; the original repository is GPL-3.0. The provenance map records the reused files.

dynlight is dual-licensed under either of

- MIT licence ([`LICENSE-MIT`](LICENSE-MIT))
- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))

at your option.
