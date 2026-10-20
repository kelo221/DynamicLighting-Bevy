# dynlight

Baked per-texel shadows with live light evaluation for [Bevy](https://bevy.org) 0.20: a port of Henry de Jongh's [Dynamic Lighting](https://github.com/Henry00IS/DynamicLighting) for Unity.

A CPU raytracer bakes, for every static triangle, which lights reach it and a shadow mask per texel (1 bit, 4 bits of soft visibility for area lights and the sun, or 8 bits of how much of the sky a texel sees). At runtime a custom material evaluates each of those lights live (color, intensity, effects, light types, cookies, shimmer) and multiplies by the baked shadow bits, so baked lights can still flicker, pulse and change color. Moving objects, realtime lights, bounce lighting, realtime shadows and volumetric fog are layered on top.

The crate is its own lighting system. It never uses Bevy's lights (`PointLight`, `SpotLight`, `DirectionalLight`, ambient light, environment maps, skyboxes, shadow maps) or `bevy_pbr`'s lighting shaders.

## Features

- **Lights:** Henry's 8 types (point, spot, discoball, wave, interference, rotor, shock, disco) plus rectangle and tube area lights (shaded from their shape, with soft baked shadows; rectangles reflect through linearly transformed cosines) and a sun (parallel light from a disk `angular_diameter` degrees wide, with soft baked shadows), 15 intensity effects, water and firelight shimmer, spot cookies. Presets: `DynamicLight::spot()`, `discoball()`, `rectangle(w, h)`, `tube(length)`, `sun()`, and `sun_light(direction)`, a sun shining along a direction.
- **Sky:** an HDR equirect image (`SkyEnvironment`) lights every surface from all around and is drawn behind the scene. It is prefiltered on the CPU when it loads: nine spherical-harmonic coefficients for diffuse light and a GGX cube for Metallic reflections. Its color, intensity and effects stay live, and turning its entity turns it. Its baked shadows store how much of the sky each texel sees, so it no longer reaches under roofs; turning the sky or swapping its image keeps the bake.
- **Baked shadows:** per-texel shadow bits for every (triangle, light) pair, with seam padding, optional alpha-tested transparency, and four filtering quality tiers.
- **Bounce light:** one baked bounce per light, stored per texel at 4 to 8 bits. Suns bounce too, and `BakeSettings::colored_bounce` colors bounce light by the surfaces it bounces off (Henry's bounce is white).
- **Dynamic geometry:** moving meshes find the lights around them through a baked light BVH and are shadowed by baked distance cubes, with angular or cubic filtering. A baked sun shadows them through its sun map, a depth map of the static scene seen from the sun, and a baked sky through a grid of sky probes.
- **Realtime lights and shadows:** lights marked `realtime` are evaluated every frame, nearest first within a budget, and each pixel only visits the realtime lights that reach its cell of a view-space grid. `ShadowMode::Realtime` adds a variance shadow cubemap so moving objects cast shadows; on a sun it adds three shadow cascades over the camera's view instead. A cubemap only redraws the faces where something changed, and a cascade only when the camera moves past its snapping step or a caster in it changes. A light or sun at its baked position leaves baked meshes out, so a still scene renders no shadow faces or cascades at all. Area lights blur their cubemaps wider.
- **Reflection probes:** `ReflectionProbe` boxes are captured into cubes at runtime, once the scene is ready. Metallic surfaces in a box reflect its cube with parallax correction, and meshes without bake data in it are lit by its irradiance instead of the ambient color.
- **Materials:** `DynLitMaterial` covers Henry's Diffuse, Metallic (GGX) and Unlit shaders, with normal, occlusion and metallic-roughness maps. glTF `StandardMaterial`s, light extras and room and portal helpers are converted on load.
- **Volumetric fog:** sphere, box, cone and square-shaft (`Frustum`) fog volumes per light, as a post-process pass. The shaft starts at an area light's size, for fluorescent fixtures.
- **Rooms and portals:** optional `Room` cells and `Portal` openings stop each camera's view at the walls, so realtime lights, shadow cubemaps and fog volumes that no camera can see through a door or window are left out. Walls are never assumed to stop light: only lights whose whole sphere is out of sight are dropped, so while every light fits the budgets the image doesn't change, and when they don't the budget goes to lights a camera can see.
- **Low-overhead scheduling:** the opt-in `SingleThreadedSchedulesPlugin` runs Bevy's engine and render-world schedules on Bevy's single-threaded executor, which saved about 1 ms a frame in the benchmark scenes and 1.0–2.3 ms in a game-like one. A game's own `Update` and fixed-timestep systems keep the multi-threaded executor unless you ask otherwise.
- **Debug views:** `DynamicLightingSettings::debug_view` shows lighting on white surfaces, or a heat map of how many lights reach each fragment.

## Compared with Henry's original

The core is Henry's design, ported closely: baking per-triangle light lists with a 1-bit shadow mask per texel, his ±2-texel padding and shadow filters, bounce light, distance cubes for moving objects, the eight light types and fifteen effects, his attenuation and his volumetric fog. The baked triangle data keeps his layout. What differs:

**New in dynlight**

- **Area lights.** Rectangle and tube lights are shaded from their shape and bake soft shadows: each texel stores how much of the shape it sees in 4 bits (16 levels) instead of Henry's single shadow bit, so penumbras are smooth and harden where objects touch the ground. Moving objects get soft shadows from the light's distance cube, with penumbras that widen with the distance to the blocker. Otherwise they follow his rules: one light at a fixed position with a `radius`, and moving, turning or resizing a baked area light falls back to realtime lighting until the next bake.
- **A sun.** `LightType::Sun` is a light at infinity with a disk (0.53° by default, like the real sun). Its baked shadows are Henry's single bit per texel for a disk of 0°, and 4 bits of soft visibility otherwise, so penumbras widen with the distance to the blocker. Moving objects get its shadows from a baked sun map. It keeps Henry's model of a light: color, intensity and effects change live, and turning a baked sun falls back to realtime lighting until the next bake. With `ShadowMode::Realtime`, moving objects cast sun shadows too, through three variance shadow cascades up to `sun_shadow_distance` (50 m) that hold only the moving meshes while the sun stands where it was baked. With `Illumination::SingleBounce` a sun bakes one bounce, gathered over each texel's hemisphere (Henry's photon cubes need a light position).
- **An HDRI sky.** Henry's scenes get a flat ambient color; `LightType::Sky` lights them from an HDR image, with reflections on Metallic materials, and draws the image as the background. It is a light like the others (color, intensity and effects change live). Its baked shadow is how much of the sky each texel sees, in 8 bits from 64 rays over the hemisphere, and it dims the sky's reflection as much as its diffuse light, so the sky stays out from under roofs. It doesn't depend on the image, so the sky can turn or change its image without a new bake. Moving objects get sky shadows from a baked grid of probes about a meter apart. Faces turned to the ground see no sky; the sky bakes no bounce light, so only a bouncing sun lights them from below.
- **Cached realtime shadows.** Shadow cubemaps redraw only the faces where something changed, and leave baked meshes out while their light stands where it was baked. Area lights blur their cubemaps wider.
- **Clustered realtime lights.** Henry's shader visits every realtime light at every pixel; dynlight lists the realtime lights per cell of a view-space grid, so with 1000 realtime lights our lit pass takes 1.5 ms of GPU time, where Henry's whole frame takes 6.3 ms (see Performance).
- **Reflection probes.** Henry's Metallic shader samples Unity's reflection probes. dynlight captures its own `ReflectionProbe`s at runtime and reflects them with box projection; they also light meshes without bake data.
- **Rooms and portals.** Henry culls realtime lights, cubemaps and fog by the camera's frustum alone; rooms also cull what no camera sees through the doors and windows.
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

Meshes whose material blends (`AlphaMode::Blend`, `Premultiplied`, `Add` or `Multiply`), such as glass, cast no shadows, baked or realtime; they still receive them. Add `NoShadowCasting` to keep any other mesh out of every shadow, for example a lamp shade with the light inside it; in Blender, set the custom property `dynlight_cast_shadows` to `False`. Changing either after a bake keeps the bake but warns that the shadows the mesh casts are out of date.

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

### Reflection probes

```rust,ignore
// A probe filling a room 8 m wide, 3 m high and 6 m deep: the box is the entity's local ±1 cube.
commands.spawn((
    Name::new("Hall probe"),
    ReflectionProbe::default(),
    Transform::from_xyz(0.0, 1.5, 0.0).with_scale(Vec3::new(4.0, 1.5, 3.0)),
));
```

A probe is captured once the scene is ready: no render pipeline compiling, the bake applied, the sky built and the lit shader's features chosen. Six 90° cameras render its surroundings for a few frames from the box's middle (or `capture_offset` from it), the faces are read back, and the environment slice's prefilter turns them into a 128² cube and nine spherical-harmonic coefficients on a background task, one probe at a time. `ReflectionProbeStatus` counts the probes still pending. A probe is captured again when it moves or changes, when a bake is applied, the sky or the lit shader changes, and when a `CaptureReflectionProbes` message is sent; send one after you change lights, since lights don't trigger a capture by themselves. Metals in a capture reflect what the probes held before it, so a reflection shows one bounce.

Each pixel uses the smallest box that holds it, and outside every box the sky. Metallic materials reflect the probe's cube with box projection: the reflected ray is followed to the box's side and the cube is sampled toward that point from the capture point, so the reflection lines up with the room's walls. This is exact for walls that lie on the box's sides, so fit the box to the room. Meshes without bake data in a box take its irradiance instead of the ambient color; baked meshes keep their bounce light and the ambient color. A capture sees emissive surfaces and the light on the walls, so a scene that also has lights for its emitters may want a lower `diffuse_intensity`. The sky reflects only outside the boxes, since a probe's cube already holds what it sees of the sky.

Limits: at most 16 probes (the smallest boxes are kept, with a warning); there is a seam where two boxes meet in a doorway, since probes aren't blended; a capture picks the realtime lights around the probe for a few frames, which can change the main view's realtime lights while it runs. In Blender, a Cube empty with the custom property `dynlight_reflection_probe = True` becomes a probe filling its ±1 cube (it may also be the `dynlight_room`), and `dynlight_probe_diffuse` sets its `diffuse_intensity`.

### Shader variants

The lit shader is compiled for `DynamicLightingSettings::quality` and for the lighting paths in `DynamicLightingSettings::shader_features`: light patterns and cookies, area lights, the sun, the sky, realtime lights, realtime shadows, meshes without bake data, the bake's light BVH and shadows on those meshes, and baked bounce light. Everything is on by default. A game that doesn't use some of them can switch them off for a faster shader:

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

### Rooms and portals

Indoor scenes can describe their rooms, so lights behind walls stop costing GPU time:

```rust,ignore
// Two rooms on either side of a wall in the plane z = 0; each cell runs to the wall's middle.
commands.spawn((Name::new("Hall"), Room::new(Vec3::new(8.0, 3.0, 4.0)), Transform::from_xyz(0.0, 1.5, 2.0)));
commands.spawn((Name::new("Kitchen"), Room::new(Vec3::new(8.0, 3.0, 4.0)), Transform::from_xyz(0.0, 1.5, -2.0)));
// The door between them, a rectangle in its local XY plane, linked to both cells automatically.
commands.spawn((Name::new("Door"), Portal::new(Vec2::new(1.2, 2.2)), Transform::from_xyz(1.0, 1.1, 0.0)));
// A window in the hall's east wall. No cell lies past it, so it opens onto the outside.
commands.spawn((
    Name::new("Window"),
    Portal::new(Vec2::new(2.0, 1.2)),
    Transform::from_xyz(4.0, 1.6, 2.0).looking_to(Vec3::X, Vec3::Y),
));
```

Every frame each camera's view is traced from the cell it stands in through the portals it can see. A realtime light, a shadow cubemap or a fog volume is then dropped only when its whole sphere of influence lies outside everything any active 3D camera (or `LightingViewer`) sees, the frustum and the walls included; for fog, any active `FogPostProcess` camera. Walls never stop light, because a light whose cubemap is missing (budget, `IntegratedGraphics`) shines through them, and so does some of a shadowed light's light near thin walls. Dropping only what can't reach a visible point keeps the image unchanged while every light fits the budgets; when they don't, the freed budget goes to lights a camera can see. Suns and skies are never dropped, and baked lights where they were baked stay packed; only their cubemaps are culled. A view starts in every cell its camera's near plane reaches, not only the one the eye is in, so a camera pressed against a wall or a door, whose near plane pokes through it, keeps the room behind.

The rules for rooms:

- Cells are boxes (under any transform, a parent's shear included) and must not overlap.
- A cell reaches the portals in its walls: at a doorway it runs to the wall's middle, where the portal is.
- Every opening a camera could look through (doors, windows, glass, gaps) needs a portal. A closed door can stay an open portal; that only culls less.
- Everything outside the cells is one outside space, so a window with no cell within a meter of it opens there.

What dynlight checks:

- **Turns portal culling off,** so the cameras' frusta cull alone, with a `dynlight portals:` warning that names the rooms and portals: cells overlapping by more than `PortalSettings::tolerance`; a portal side that two cells cover, or that a cell within a meter touches without covering the whole opening; flat, singular or non-finite transforms; a `PortalSide::Into` naming nothing or an entity that isn't a `Room`.
- **Warns and keeps culling:** a portal whose two sides lead to the same place, so it joins nothing.
- **Can't detect, and can cull visible lights:** a missing portal; a cell reaching past real walls; part of a room left outside every cell; a side meant to open into a room that finds no cell within a meter of it, so it opens onto the outside.

Set a side yourself with `Portal { front: PortalSide::Into { rooms: vec![hall], outside: false }, .. }` (the front is the portal's local +Z side); listing more rooms than the side really opens into is always safe. Orthographic cameras and views more than `PortalSettings::depth_limit` (16) portals deep cull nothing either. In the `rooms` example's 6 × 6 grid of rooms with every light within the budgets, portal culling keeps 15 of 101 realtime lights and 7 of 36 shadow cubemaps for the same image. With the lamps moving, so every cubemap redraws, the frame drops from 19.3 to 6.3 ms (1920 × 1080, RTX 4080 SUPER, Direct3D 12); still scenes, whose cubemaps are cached, gain little. The culling itself costs microseconds a frame (`cargo bench --bench portals`). `PortalSettings::enabled` switches portal culling off, `PortalStats` reports what it did last frame, and with `LightGizmos` on the cells and portals are drawn (portals in red when they turned culling off, closed ones dimmed).

#### Doors

Add `ClosedPortal` to a portal while its door or window is fully shut, and remove it in the frame the door starts to open: `commands.entity(door).insert(ClosedPortal)`. Views then stop at it. Lights are still culled by their whole sphere, so a light whose sphere reaches through a shut door into a view stays. Keep the portal fixed in the doorway and animate a separate door mesh. A change made in `Update` or earlier takes effect in the same frame without rebuilding the room graph; a system that changes it in `PostUpdate` must run `.before(PortalSystems::Trace)`. Doors can change whether a view stays within the trace's limits: closing one can bound a view that was too deep, and opening one can do the opposite. In the `rooms` example's 6 × 6 grid, shutting every door cuts the realtime lights kept from 15 to 9 and the cubemaps from 7 to 4; what stays are the lights whose spheres reach the part of its own room the camera sees, some of them through the walls.

#### Rooms and portals from Blender

glTF node extras (Blender custom properties, exported with Include → Custom Properties) can place rooms and portals:

- `dynlight_room = True`: the object's local ±1 cube becomes a `Room`. Use a Cube empty with display size 1 and size it only with the object's scale. Don't apply the scale: on an empty it goes into the display size, which isn't exported. A mesh works too while its geometry stays the ±1 cube.
- `dynlight_portal = True`: the ±1 square in the object's local X–Z plane becomes a `Portal`, with the width along Blender's X, the height along Z and the thin side along Y (Blender's +Y becomes glTF's −Z). Both sides are found automatically.
- `dynlight_portal_closed = True` on the portal object: the portal starts closed. Put it on the opening's helper, not on the door mesh, which is a separate object.

These keys take only `True`/`False` or `1`/`0`. A helper's own mesh is removed when the scene spawns, so it never renders, casts shadows or bakes, and its light, fog and `dynlight_static` keys are ignored; child objects are kept. Mistakes are logged as `dynlight gltf:` warnings naming the node, and each one leaves culling keeping more, never less: a room that is also a portal makes neither, an unreadable room key makes no room, and an unreadable portal or closed key makes an open portal. A mesh helper whose geometry isn't the unit shape is reported too. A node becomes a light only with one of the light keys listed in `gltf_import::extras`; an unknown `dynlight_` key is reported and no longer makes a default point light. Room bounds aren't computed from the meshes; place the boxes yourself. [`tests/fixtures/rooms_gltf.py`](tests/fixtures/rooms_gltf.py) builds a small scene this way.

### Scheduling

In a scene like the benchmark hall below, most of a frame's CPU time goes to Bevy's multi-threaded executor rather than to the systems it runs. A frame runs several hundred systems, most of them for a few microseconds, and the executor hands them to worker threads and waits for them to come back. `SingleThreadedSchedulesPlugin` runs Bevy's engine schedules of the main app and the render world's schedules on Bevy's single-threaded executor instead:

```rust,ignore
app.add_plugins((DefaultPlugins, DynamicLightingPlugin, SingleThreadedSchedulesPlugin::default()));
```

This doesn't make rendering single-threaded. Bevy's per-mesh work (extracting and preparing meshes, writing the instance and indirect buffers, finishing command buffers) is split across the task pool inside single systems, and keeps that. Each pass records its draws in one system on either executor, and Bevy runs the prepass and main-pass systems one after another anyway. What the plugin gives up is running whole systems side by side. It costs little for Bevy's engine and render systems, which are short. A game's own systems can be long and independent (physics, AI, animation), and that is what the multi-threaded executor is for.

So `main` chooses how much of the main app switches:

- `MainSchedules::Engine` (the default) switches `First`, `PreUpdate`, `StateTransition`, `RunFixedMainLoop`, `SpawnScene`, `PostUpdate` and `Last`. `Update` and the fixed-timestep schedules keep the multi-threaded executor.
- `MainSchedules::All` also switches `Update` and `FixedFirst` to `FixedLast`, for apps whose own systems are light. The examples' `--single-threaded` uses it.
- `MainSchedules::Off` leaves the main app alone. Use it when a plugin of yours runs heavy systems in `PreUpdate` or `PostUpdate`; some physics plugins step there.

Most of the saving is in the render world: in a game-like scene, switching only the render world saved 0.8–1.1 ms of the default's 1.0–1.4 ms. `render: false` leaves the render world alone.

The render world still runs on its own thread beside the main world, systems that split their own work across the task pool (`par_iter`, as transform propagation and visibility do) still do, and commands are applied at the same points. The one exception is an exclusive system with commands of its own, such as one piped into a system with `Commands`: they are applied at the next sync point instead of when it returns. What changes is that systems Bevy may run in either order now run in one order, fixed when the schedule is built, and each schedule runs all its systems on its own thread. dynlight renders the same images either way (the visual baselines are checked with the plugin), and Bevy's ambiguity detection finds no pair involving dynlight systems whose result depends on that order.

Schedules your app adds, and schedules created after the plugin's `finish`, keep their executor and settings. A render-world system of your own that needs the main thread (`NonSend` data created there) loses it. Bevy's own window surfaces need it on macOS and iOS, so there the `Render` schedule keeps its executor. The examples take `--single-threaded`.

`DynamicLightingPlugin` doesn't add it, because the executor is a choice for the whole app. It saved 0.9–1.0 ms a frame in dynlight's benchmark scenes and 1.0–2.3 ms in a game-like scene with many materials, two cameras, moving meshes and 2 ms of game logic, for dynlight and for Bevy's own lights alike (see Performance). There `All` put the game logic on the main thread; with 4 ms of it, that thread set the frame and `All` was 0.25 ms slower than the default. Measure your own app with and without it.

### Platforms

- **Primitive indices:** baked shadows are looked up by triangle. On GPUs with wgpu's `PRIMITIVE_INDEX` feature (desktop Vulkan, DX12, Metal) this is free. Elsewhere, baked meshes are de-indexed when they load and carry a triangle-ID vertex attribute instead. `DynamicLightingSettings::triangle_ids` forces either path.
- **Shadow cubemaps:** use `Rg32Float` where the GPU can filter 32-bit floats, and `Rg16Float` otherwise.
- **Web:** WebGPU works, through the triangle-ID path (wgpu doesn't expose primitive indices on the web). Bakes run on the browser's main thread with `BakeRequest { output: None, .. }`. WebGL2 is not supported, because it has no storage buffers in fragment shaders.

## Examples

| Example | What it shows |
|---|---|
| `cargo run --example lighting_test` | Every light type, effect, shimmer, a cookie, fog volumes and bounce light, a fluorescent ceiling panel with a fog shaft and soft baked shadows, a wall tube, and a cube to drive around with a realtime-shadowed lantern. `--bake` (or B) bakes the scene; `--triangle-ids` uses the fallback path. |
| `cargo run --example realtime_shadows` | Orbiting point and spot lights with realtime shadow cubemaps over moving crates; logs the cube faces rendered per frame. `--still` stops everything, so no face renders. |
| `cargo run --release --example stress_lights -- --bake` | 1000 baked lights among about 1000 pillars; `--realtime` makes them all realtime (and drifting) instead. |
| `cargo run --example outdoor -- --bake` | A courtyard under a baked soft sun and a baked procedural sky, with a porch, a pergola, trees, a lamp post and two metal balls reflecting the sky; a rolling ball and a spinning crate take the sun's shadows from the sun map and the sky's from the sky probes, and cast theirs through the sun's realtime cascades. Later runs load the bake. `--hdri path.exr` lights it with an HDR image instead, turned so its sun matches the sun light. |
| `cargo run --example rooms` | A corridor with six rooms and a window onto a yard, each room with a shadowed lamp, cone fog in one, culled through the doors. P (or `--no-portals`) switches portal culling, G the room gizmos, C the doors (`--closed-doors` starts with them shut); `--probes` adds a reflection probe and two metal objects to every room; `--path` flies a fixed route, `--grid N` builds N × N rooms, `--budgets` raises the light budgets. Logs the lights, cubemaps and fog kept once a second. |
| `cargo run --example gpu_check` | Reads the rendered image back and compares it with the CPU reference of the light shaders. Exits with an error on a mismatch. |
| `cargo run --example inspector --features inspector` | A Bevy Feathers panel for editing one light live and baking with a button. |

All examples except `gpu_check` accept `--frames N` (exit after N frames) and `--screenshot path.png`.

The GPU checks need a GPU, so they are skipped by `cargo test`; run them with `cargo test --test gpu_examples -- --ignored --test-threads=1`.

Visual baselines guard the look: `cargo test --test gpu_baselines -- --ignored --test-threads=1` renders still frames of `lighting_test`, `outdoor`, `realtime_shadows`, `rooms` and `stress_lights` (`--capture path.png` freezes time and waits for the bake and the shaders) and compares them with `tests/baselines/` using NVIDIA's [FLIP](https://github.com/NVlabs/flip) metric. A failure leaves the new frame and an error map in `target/tmp/baselines/`. After an intended change to the look, rerun with `DYNLIGHT_BLESS=1` and review the new images before committing them.

## Performance

dynlight against Bevy's own lights and against Henry's package in Unity, in the same scene: the `stress_lights` layout, about 1000 pillars in an 80 m hall. Both harnesses live outside this repository: one spawns Bevy's `PointLight`s, the other is a Unity project. Unity builds each scene from the harness's scene description and bakes it with Henry's own baker.

Setup: 1920×1080, 4× MSAA, RTX 4080 SUPER, Direct3D 12 for all three, 512² shadow cubemaps. dynlight and Bevy use the release profile; Unity 6000.6.4f1 uses an IL2CPP release player, the Built-in pipeline and Henry's package at e2a51ad. Each figure is the median of 10 runs, with all three engines alternating in one session on 2026-10-19. The windows open without taking focus; Unity's player took it anyway in 7 of its 50 runs, with the same times as the rest.

| Scene | Frame: dynlight | Frame: Bevy | Frame: Henry | GPU: dynlight | GPU: Bevy | GPU: Henry |
|---|---|---|---|---|---|---|
| 1000 still lights (dynlight and Henry: baked shadows; Bevy: no shadows) | 2.12 ms | 2.27 ms | 0.76 ms | 0.69 ms | 1.33 ms | 0.46 ms |
| 100 moving realtime lights, no shadows | 2.21 ms | 2.53 ms | 0.41 ms | 0.36 ms | 0.27 ms | 0.26 ms |
| 1000 moving realtime lights, no shadows | 2.22 ms | 2.34 ms | 6.50 ms | 1.54 ms | 1.32 ms | 6.32 ms |
| 15 of 100 lights with realtime shadows, scene still | 2.41 ms | 25.6 ms | 7.05 ms | 0.34 ms | – | 2.20 ms |
| 15 of 100 lights with realtime shadows, scene moving | 7.99 ms | 26.1 ms | 7.09 ms | 0.31 ms | – | 2.32 ms |

The GPU columns don't measure the same thing. dynlight's and Bevy's are the main lit pass alone (wgpu timestamps); the harness doesn't report it for Bevy's shadowed lights, most likely because their 90 shadow views use up the 256 timestamps Bevy records per frame. Henry's is Unity's GPU time for the whole frame. The shadow rows use 15 lights because Henry's package turns realtime shadows off for good after a frame that fills all 16 of its cubemaps.

Bevy doesn't sort opaque draws front to back, and its draw order changes from build to build, so the same scene can shade up to 30% more fragments in another build of either engine. The lit-pass numbers carry that uncertainty. The build measured here draws the 1000-pillar hall in a good order: it shades 3.0 M fragments, about as many as Bevy's 2.9 M, where the previous measurement's build shaded 3.8 M. That is why its 1000-light rows read lower than then (0.88 and 1.89 ms). A `DepthPrepass` on the camera makes each pixel shade once, whatever the order; with one, this build's baked pass takes 0.64 ms and its 1000 realtime lights 1.35 ms, as the previous build's 0.62 and 1.37 ms.

- **Frame time** in dynlight and Bevy is mostly engine overhead: 2.1–2.4 ms for dynlight and 2.3–2.5 ms for Bevy's own lights, about half of it Bevy's multi-threaded executor (see below). dynlight saves a little by not running Bevy's GPU light clustering, which it doesn't need. Unity's player needs about 0.4 ms.
- **Baked lights** shade faster than Bevy's lights without shadows, and their shadows cost nothing at runtime. Like Henry's package, dynlight compiles its lit shader for the quality tier and the material's shading model. Switching off the lighting paths this scene doesn't use (`shader_features`, or `FeatureSelection::Automatic`, which picks the same paths here; see Usage) brings the pass to 0.50 ms, close to Henry's 0.46 ms for the whole frame.
- **Realtime lights** are sorted into the cells of a view-space grid, as Bevy does, so a pixel only visits the lights that reach it. Henry's shader visits every realtime light at every pixel, so his 1000-light frame takes four times our lit pass. The table uses the default shader, which compiles every lighting path; with `FeatureSelection::Automatic` the lit pass takes 0.25 ms for 100 realtime lights and 0.75 ms for 1000, against Bevy's 0.27 and 1.32 ms.
- **Realtime shadows** are cached in dynlight: a cubemap face only redraws where something changed, so a still scene renders none. Bevy's and Henry's still scenes cost as much as their moving ones. When everything moves, all three redraw 90 faces per frame. Unity does that in about 7 ms, and Bevy, which pays a per-view cost for every face, in 26 ms. dynlight draws every shadow view from one driver in 8.0 ms, 1.9 ms of it GPU time for the faces.
- **The images match.** Rendered without tonemapping from the same camera, dynlight's and Henry's frames of the stress, light-type and shadow scenes differ on average by 0.01–0.14 of one 8-bit step. At most 0.2% of pixels differ by more than 16 steps, all along pillar edges.

### With `SingleThreadedSchedulesPlugin`

Frame times without the plugin, with its default (`MainSchedules::Engine`: `Update` and the fixed-timestep schedules stay multi-threaded) and with `MainSchedules::All`, for dynlight and for Bevy's own lights with the plugin added to the same harness. The setup is the one above, measured in a later session on 2026-10-20; each figure is the median of 10 runs, with every configuration alternating in one session.

| Scene | dynlight | default | `All` | Bevy | default | `All` |
|---|---|---|---|---|---|---|
| 1000 still lights | 1.99 ms | 1.12 ms | 1.12 ms | 2.12 ms | 1.67 ms | 1.69 ms |
| 100 moving realtime lights | 2.02 ms | 1.15 ms | 1.12 ms | 2.16 ms | 1.19 ms | 1.27 ms |
| 1000 moving realtime lights | 1.99 ms | 1.84 ms | 1.82 ms | 2.28 ms | 1.70 ms | 1.66 ms |
| 15 of 100 lights with realtime shadows, scene still | 2.01 ms | 1.13 ms | 1.10 ms | 23.0 ms | 14.5 ms | 14.6 ms |
| 15 of 100 lights with realtime shadows, scene moving | 7.77 ms | 6.73 ms | 6.84 ms | 23.0 ms | 14.3 ms | 14.5 ms |

- **The plugin saves 0.9–1.0 ms a frame** in dynlight's light scenes, 0.5–1.0 ms in Bevy's, and over a third of Bevy's shadowed frame, which pays a per-view cost for each of its 90 shadow views. dynlight's 1000 realtime lights save only 0.15 ms, because that frame is GPU-bound: its lit pass takes 1.53 ms with every lighting path compiled. (In the previous session, `FeatureSelection::Automatic` brought that pass to 0.88 ms and the frame to 1.36 ms.)
- **The default and `All` are within about 0.1 ms of each other** in every scene: these scenes run almost nothing in `Update`.
- **Launches spread widely in this session**, up to 30–50% above the median in a few rounds, for every configuration alike. Without the plugin, its frames read 0.1–0.4 ms lower than the table above.
- **The render thread sets the frame** in every scene: it is busy for all but 0.1–0.3 ms of it. With the plugin, dynlight's render thread is busy for about 1.0 ms of the 1.1 ms frames of the lighter scenes, and its main thread for 0.3–0.7 ms. So caching dynlight's per-frame light packing on the main thread wouldn't shorten the frame, and uploading the light buffer only when it changes saved nothing measurable.
- **GPU times aren't comparable across executors.** In the light scenes the lit pass reads lower with the plugin for both engines (100 moving lights: dynlight 0.37 → 0.21 ms, Bevy 0.28 → 0.17 ms) with the same shaders, most likely because the GPU clocks higher when frames come faster. Compare GPU times under one executor.
- Limiting Bevy's compute task pool (`TaskPoolPlugin`) to 4 threads moved the frame by −0.2 to +0.1 ms in the light scenes, with no consistent direction, in a shorter screen of 4 runs; it isn't in the table.

#### In a scene more like a game

The benchmark hall is the plugin's best case: one material, one camera and almost no game logic. The harness's `--game` option adds to it what a game has more of. The pillars get 32 materials instead of one. 200 cubes without bake data move above the floor, with 8 materials of their own, or one each in the 232-material rows. A second camera draws a 640 × 360 picture-in-picture view. Eight independent `Update` systems each work for 250 µs a frame (500 µs in the last row), 2 ms of game logic that the multi-threaded executor can spread over its threads. "Render only" is `main: MainSchedules::Off`. Same session and method as above.

| Scene | No plugin | Render only | Default | `All` |
|---|---|---|---|---|
| dynlight, 100 moving realtime lights, 40 materials | 4.32 ms | 3.39 ms | 3.33 ms | 3.25 ms |
| Bevy, 100 moving lights, 40 materials | 4.64 ms | 3.57 ms | 3.29 ms | 3.27 ms |
| dynlight, 1000 baked lights, 40 materials | 4.31 ms | 3.40 ms | 3.30 ms | 3.25 ms |
| dynlight, 100 moving realtime lights, 232 materials | 9.55 ms | – | 7.30 ms | 6.99 ms |
| Bevy, 100 moving lights, 232 materials | 8.75 ms | – | 7.67 ms | 7.85 ms |
| dynlight, 100 moving realtime lights, 40 materials, 4 ms of game logic | 6.00 ms | 5.21 ms | 4.71 ms | 4.96 ms |

- **The plugin still saves 1.0–2.3 ms a frame** in a game-like scene, for dynlight and Bevy alike, and more where the render world has more to do. Most of it comes from the render world.
- **`All` moves the game logic onto the main thread.** With 2 ms of it, the main thread is busy for 2.4–2.6 ms a frame under `All` against 0.7–0.9 ms with the default, about as long as the render thread, so the frame doesn't get longer yet. With 4 ms of game logic it is busy for 4.6 ms and sets the frame, and `All` is 0.25 ms slower than the default.
- **Many materials cost the render world in both engines.** Giving each of the 200 moving cubes its own material adds about 2.4 ms to the render thread with one camera, and the second camera about doubles it. That is Bevy's per-material and per-view work, and dynlight and Bevy pay it alike.

Baking the 1000-light scene (1047 meshes, 6.3 M texels, 18 M rays) takes 1.3 s on a 32-thread desktop CPU; Henry's baker takes 22.2–22.6 s for the same scene in the Unity editor (direct light only for both, 16 texels per metre). `cargo bench --bench bake` measures the baker on its own (about 27 M shadow rays per second), and `cargo bench --bench clusters` the per-frame light grid (0.27 ms for 1000 lights).

## Development

Contributor rules (slice layout, 400-line file limit, what may and may not be ported) are in [`AGENTS.md`](AGENTS.md); the design and milestones are in [`PLAN.md`](PLAN.md). The `dev` feature links Bevy dynamically for faster rebuilds; the aliases in `.cargo/config.toml` enable it: `cargo check-dev`, `cargo test-dev`, `cargo run-dev --example lighting_test`, `cargo clippy-dev`.

### Building the web demos

Install the `wasm32-unknown-unknown` target, the `wasm-bindgen-cli` version that matches `wasm-bindgen` in `Cargo.lock` (the build script checks it) and, optionally, Binaryen's `wasm-opt`. Then run `bash web/build.sh` and serve `dist/`, e.g. `python -m http.server 8080 -d dist`. The script uses the size-optimized `wasm-release` profile, never the `dev` feature. GitHub Pages builds and deploys the same site on every push to `master`.

## Credits and licence

The lighting design, formulas and shaders are Henry de Jongh's, ported from [Henry00IS/DynamicLighting](https://github.com/Henry00IS/DynamicLighting) under the MIT licence; his notice is in [`LICENSES/DynamicLighting-MIT.txt`](LICENSES/DynamicLighting-MIT.txt). The shadow blur kernel comes from Daniel Ilett's SmoShaders (MIT), notice in [`LICENSES/SmoShaders-MIT.txt`](LICENSES/SmoShaders-MIT.txt). Rectangle-light highlights use the fitted tables of Heitz, Dupuy, Hill and Neubelt, "Real-Time Polygonal-Light Shading with Linearly Transformed Cosines" (SIGGRAPH 2016), BSD-2, notice in [`LICENSES/LTC-BSD-2.txt`](LICENSES/LTC-BSD-2.txt). Rooms and portals follow the design of Godot 3's rooms and portals by lawnjelly (MIT); no code is taken from it. [`docs/PORT_MAP.md`](docs/PORT_MAP.md) lists where every ported file comes from. Nothing is taken from Unity's own code; where Henry relied on Unity (random numbers, Perlin noise, camera rendering), dynlight uses its own implementations.

Code reused from the user-owned [bevyout](https://github.com/kelo221/bevyout) `DynamicLightsPort` branch is relicensed by its sole author under this crate's MIT OR Apache-2.0 terms; the original repository is GPL-3.0. The provenance map records the reused files.

dynlight is dual-licensed under either of

- MIT licence ([`LICENSE-MIT`](LICENSE-MIT))
- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))

at your option.
