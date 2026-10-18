# DynLight: porting Henry de Jongh's Dynamic Lighting to Bevy 0.20

Status: M0 done (2026-10-15); M1 next. Revised 2026-10-15 against Bevy 0.20.0 (released 2026-10-08).

## 1. Goal and approach

Port [Henry00IS/DynamicLighting](https://github.com/Henry00IS/DynamicLighting) (v1.331, MIT) to a standalone Bevy 0.20 crate:

- **Shaders:** Henry's HLSL becomes WESL.
- **Runtime and baker:** Henry's C# becomes Rust.
- **Reuse:** the user-owned prior port in `kelo221/bevyout` (branch `DynamicLightsPort`, `src/vsa/dynamic_lighting/`) is reused where it is sound.

Henry's design is ported as-is:

- **Baking:** a CPU raytracer stores, per static triangle, the list of lights that reach it and a 1-bit-per-texel shadow mask per (triangle, light); area lights store 4 bits of soft visibility per texel (§12, Beyond Henry).
- **Runtime:** a custom material looks up the triangle by primitive index and evaluates each listed light live (color, intensity, effects, light types, cookies, shimmer), multiplied by the sampled shadow bits.
- **Moving objects:** they use a light BVH plus baked distance cubes.
- **Moving lights:** they use a realtime budget, optionally with variance shadow cubemaps.
- **Extras:** baked single bounce and volumetric fog are optional.

Reference checkouts (not part of this repo) are in `C:\Users\V\Documents\dynlight-refs\`:

- `DynamicLighting/` holds Henry's source. Below, `S/` = `AlpacaIT.DynamicLighting/Shaders/` and `C/` = `AlpacaIT.DynamicLighting/Scripts/`.
- `bevyout/` is a sparse checkout of the prior port; `BO/` below = `bevyout/src/vsa/dynamic_lighting/`.

## 2. Rules

| Source | Rule |
|---|---|
| Henry de Jongh's code (MIT) | Port directly. Ship his MIT notice and keep his in-file copyright headers (e.g. the 2025 angular filter, `S/DynamicLighting.cginc:468`). |
| `bevyout` DynamicLightsPort | User-owned (single author), so reuse and relicense freely. |
| **Unity's own libraries and engine behavior** | **Never duplicate.** Replace each item with a Rust/Bevy equivalent (table below). |
| **Bevy lights** | **Never use.** No `PointLight`/`SpotLight`/`DirectionalLight`, ambient, environment maps, lightmaps, shadow maps, Solari, `StandardMaterial` lighting, `bevy_pbr` lighting/shadow shader modules, and no vendored/patched `bevy_pbr`. |
| Third-party snippets inside Henry's package | See the last four rows of the table below. |

### Unity items and their replacements

| Unity item | Used in Henry / bevyout | Replacement |
|---|---|---|
| `UnityEngine.Random` (Xorshift128, `InitState`) | Flicker, Random, FluorescentRandom effects; bake dither; bounce `onUnitSphere`; `BO/core/unity_random.rs` | Our own PCG32 / SplitMix stateless hash, seeded per (light id, step) or per (mesh, light, texel, sample) for deterministic parallel bakes |
| `Mathf.PerlinNoise` | Candle, Fire, Generator, Lightning, Cloudy, Overcast; `BO/core/unity_math.rs:1-15,48-108` | Our own 2D gradient noise, remapped to [0,1] around 0.5 and calibrated so threshold probabilities (P(n>0.5/0.7/0.74/0.8)) give the intended look. Tested statistically, not against Unity output |
| `Mathf` semantics (`Lerp` clamps, `RoundToInt` banker's rounding, `Clamp` argument order) | effects, packing | Small helpers in `shared::math` named for what they do (`lerp_clamped`, …) |
| `Unity.Mathematics`, `UMath` | everywhere | `glam` (via Bevy) |
| `Physics` / `RaycastCommand` / MeshColliders | bake rays | Our own BVH ray queries (`obvhs` or the `bvh` crate) + `rayon`; two-sided hits, `t ≤ maxDistance` |
| Jobs / Burst / `NativeArray` / `TransformAccessArray` | bake and runtime | `rayon`; Bevy queries on `GlobalTransform` |
| `Camera` renders (photon cubes, distance cubes, shadow camera) | bake and runtime | Bake: CPU closest-hit rays from the light (deterministic, headless). Runtime: our own `Core3d` render pass |
| `ComputeBuffer`, `MaterialPropertyBlock`, `Shader.SetGlobal*`, keywords | runtime | `ShaderBuffer`, material bind group, `MeshTag`, WESL `@if` flags via `Material::specialize` |
| `ScriptableObject` (`RaycastedScene`) + editor serialization + InstanceID ordering | bake storage | Our own `.dynlight` asset (`AssetLoader`) + stable `BakeId` components |
| Editor APIs (menus, toolbar, gizmos, draw modes, progress bar) | editor | Bevy systems, gizmos, a debug-view resource, optional Feathers UI |
| Unity shader includes and macros (`UnityCG.cginc`, `UnityObjectToClipPos`, `unity_ObjectToWorld`, `_WorldSpaceCameraPos`, `_Time`, `TRANSFORM_TEX`, fog/instancing/lightmap/specCube macros, ForwardAdd/ShadowCaster passes) | `S/*` | Bevy `mesh_functions`, the view uniform, our own globals; dropped where Unity-only (§7.3) |
| Unity's left-handed conventions (cube face tables, `forward`, gather swizzles) | photon/distance cubes, spot, cookies | Re-derived for Bevy's right-handed, −Z-forward space |
| Unity reference harness + golden JSON produced by running Unity | `BO/reference_unity/*`, `BO/tests/golden/*`, parity tests | Delete. Replace with analytic tests and CPU-mirror tests |
| LearnOpenGL BRDF code (CC BY-NC 4.0) reproduced in `S/Internal/Common.cginc:290-343`, `S/DynamicLightingMetallic.shader:295-344` | Metallic | Re-derive GGX / Smith / Schlick from the published formulas under our own names |
| Shadertoy `rand` (Shadertoy default licence CC BY-NC-SA) in `Common.cginc:38-41` | shimmer | Our own hash |
| SmoShaders Gaussian blur (MIT, © 2019 Daniel Ilett) in `S/Operations/GaussianBlur.shader` | VSM blur | Port and ship its MIT notice |

Our crate license: MIT OR Apache-2.0, like the Bevy ecosystem. Henry's and SmoShaders' MIT notices go in `LICENSES/`. Henry is credited in the README.

## 3. Crate layout: Vertical Slice Architecture

The project is a single crate, `dynlight` (the package is renamed in M0), organized as vertical slices. `AGENTS.md` is the source of truth for the layout, the slice rules and the 400-line file limit. This table shows where the ported and lifted code goes:

| Slice | Contents | Lifted / ported from |
|---|---|---|
| `shared/` | `BakeId`, GPU light ABI + packing, math helpers, WESL `types` / `math` / `bindings` | `BO/render/gpu.rs:30-220`, `BO/core/unity_math.rs:18-46` (renamed), `S/DynamicLighting.cginc` structs, `S/Internal/Common.cginc` |
| `light/` | `DynamicLight`, enums, presets, light buffer, realtime budget, moved lights, `light_types.wesl`, CPU mirror | `BO/core/{config,types,spatial}.rs`, `BO/bevy_bridge/mod.rs`, `C/Core/DynamicLightManager.cs`, `C/Lighting/*` |
| `light_clusters/` | Cell grid over the realtime-selection camera, per-cell realtime light lists, `clusters.wesl` (M17) | new |
| `effects/` | 15 intensity effects, our noise + RNG, fixed timestep | `BO/core/{runtime,fixed_timestep}.rs`, `C/Core/LightEffects/*` |
| `surface/` | `DynLitMaterial` variants, prepass, BRDF, `light_processor.wesl`, `lighting.wesl` | `S/DynamicLighting{Diffuse,Metallic,Unlit}.shader`, `S/Generators/LightProcessor.cginc` |
| `bake/` | Tracer steps (§8), `.dynlight` format, `BakeRequest` + progress | `C/Core/DynamicLightingTracer*.cs`, `DynamicTrianglesBuilder.cs`, `Acceleration/BvhLightStructure.cs`, `BO/baker.rs` codec |
| `baked_lighting/` | Asset loading, buffers, receiver records, primitive-index path + fallback, `triangles.wesl` | `S/DynamicLighting.cginc:608-962`, `BO/viewer/scene.rs` |
| `dynamic_geometry/` | Light BVH + distance cubes at runtime; sun maps (M19) and sky probes (M21) | `S/DynamicLighting.cginc:358-566,969-1148`, `C/Core/DynamicLightManager.DistanceCubes.cs` |
| `realtime_shadows/` | VSM cube faces (own render phase, M18) + blur; the sun's cascades (M22) | `C/Core/DynamicLightManager.ShadowCamera.cs`, `S/Operations/{ShadowDepth,GaussianBlur}.shader` |
| `cookies/` | Cookie texture array | `C/Core/DynamicLightManager.LightCookie.cs` |
| `environment/` | Sky light from an equirect image: CPU prefilter (SH9 irradiance, GGX specular cube), sky background pass (M20) | new |
| `fog/` | Volumetric fog pass + CPU mirror | `BO/render/pipeline.rs:149-181,300-362`, `BO/core/volumetric.rs`, `S/DynamicLightingPostProcessing.shader` |
| `gltf_import/` | glTF extras → `DynamicLight`, material swap | new |
| `debug/` | Gizmos, debug views, diagnostics | `C/Lighting/DynamicLight.cs:502-748` (gizmo ideas) |

Repo-level folders: `examples/` (`lighting_test` migrated from bevyout, effects gallery, baked room, dynamic objects), `tests/`, `benches/`, `assets/`, `LICENSES/` (Henry MIT, SmoShaders MIT), and `docs/PORT_MAP.md` (provenance per ported file).

## 4. Authoring API (Bevy side)

Henry's single `DynamicLight` MonoBehaviour is split into optional components, so a light only carries what it uses (implemented in M0):

- **`DynamicLight`** (`light/`, `#[require(Transform, Visibility)]`, `Reflect`):
  - Fields and defaults mirror Henry's `C/Lighting/DynamicLight.cs`: color, intensity 2, radius 4, falloff, `realtime: bool` (Henry's realtime channel), type, shadow mode, illumination, transparency, cutoffs 26°/30°, wave/rotor/disco params, bounce (`LightBounce`: color/modifier/intensity/samples/compression), shimmer + scale + modifier. The cookie handle arrives with the `cookies/` slice in M1.
  - Enums match Henry's: shadow is `Raytraced`/`Realtime`; transparency is `Disabled`/`Enabled`/`EnabledMax`; `Shimmer` is `None`/`Water`/`Random`.
- **`LightEffect`** (`effects/`): kind + pulse speed/modifier/offset + timestep 1/30. It requires `LightEffectState` (current intensity, fixed-timestep accumulator, per-light RNG seeded from the explicit `BakeId`, else from the `Name` path and local position; never from the entity, whose id shifts when anything is spawned earlier).
- **`VolumetricLight`** (`fog/`): shape (`Sphere`/`Box`/`ConeZ`/`ConeY`), radius, thickness, intensity, visibility. A light without it has no fog.
- **`BakeStatic`** (`bake/`): the static flag. It means "casts and receives baked shadows", like Henry's `isStatic` + `shadowCastingMode != Off`.
- **`BakeId(u64)`:** stable identity for lights and meshes. It is derived from the `Name` path + mesh handle when absent. It replaces Unity's InstanceID ordering and object references, and makes BVH order, mesh mapping and the "light moved from its baked origin" check deterministic.
- **Resources:** Henry's manager settings are split the same way:
  - `DynamicLightingSettings` (`shared/`): ambient, realtime light budget (32), shadow cubemap budget, quality tier, the lighting paths compiled into the lit shader (`shader_features`, §7.3), dynamic-geometry mode, tracking mode, global random seed. The volumetric budget and debug view are added with their slices.
  - `BakeSettings` (`bake/`): pixel density (Henry's default 128 px/m), max lightmap size, default bounce compression.
- **Presets:** functions that produce Henry's "special lights" (Candle, Fire, Discoball, Rotary Warning, …) with his values (`AlpacaIT.DynamicLighting.Editor/Editor/Interface/SpecialLights/*`).
- **glTF:** `DynamicLightingGltfPlugin` does the following:
  - Loads with `GltfLoaderSettings { load_lights: false }`.
  - When the scene is ready, swaps `StandardMaterial` for `DynLitMaterial` (copies textures and colors).
  - Creates `DynamicLight`s from node extras (Blender custom properties).
  - Can optionally read `KHR_lights_punctual` via the `gltf` crate, so no Bevy light component is ever created.

## 5. GPU data

These are Henry's layouts, kept unless noted. Field-level detail lives in the source; the essentials are below.

**Light struct, 112 bytes** (`S/DynamicLighting.cginc:54-80`, `C/Core/ShaderDynamicLight.cs`).

```
position: vec3, radius_sqr (-1 = disabled)
channel: u32 (flag bits), intensity, gp1, gp2
color: vec3, gp3
up: vec3, shimmer_scale
forward: vec3, shimmer_modifier
volumetric_intensity, volumetric_visibility, cookie_index: u32, shadow_cubemap_index: u32
bounce_color: vec3, falloff              <- reordered: WGSL vec3 alignment would push Henry's
                                            falloff-then-bounceColor order to 128 bytes
```

- **Channel flag bits:**
  - bit 5: realtime
  - bits 6–9: type
  - bits 10–11: shimmer
  - bit 15: shadow cubemap; on a sun, the sun has the realtime cascades (M22), and `shadow_cubemap_index` is unused
  - bit 16: cookie
  - bits 17–19: bounce bits-per-pixel − 1
  - bits 20–22: baked shadow bits per texel − 1 (1 for Henry's lights and hard suns, 4 for area lights and suns with a disk, 8 for baked skies; Beyond Henry, §12)
- **Area lights** are one record at the shape's center (`position`); they take no cookies.
- **`gpN` per light type:** follows `C/Core/DynamicLightManager.cs:1322-1420`.
- **Suns** (M19): `forward` points towards the sun, `gp1`/`gp2` hold the cosine and sine of the disk's angular radius, and `radius_sqr` is `f32::MAX` (no attenuation).
- **Reuse:** lift the ABI, packing and offset tests from `BO/render/gpu.rs`, and check that its field order matches the reordered layout.

**Light buffer contents.**

- Baked lights come first, in light-BVH order (the bake reorders them), then the baked suns by `BakeId`, then the baked skies by `BakeId` (M21); realtime lights follow, realtime suns and skies first. Suns and skies are in neither the light BVH nor the realtime light grid.
- A globals header comes first in the same buffer (48 bytes). It holds the baked count, realtime count, ambient, time (our own clock; Bevy's `globals.time` wraps at 3600 s), the quality level, and how many of the baked and realtime lights are suns (`baked_distant`, `realtime_distant`). That saves a binding, and globals change every frame, which a per-material uniform would make expensive.
- The sky block follows (M20, 144 bytes): the sky's nine spherical-harmonic coefficients of irradiance over π, one `vec4` each, in the sky's own frame. It holds zeros until a sky is built, and the shaders read it only for an enabled sky light record. The sky's light record (`LightType::Sky`) is a distant light like a sun, baked (M21) or realtime; its color, intensity and turn apply to the coefficients at shading time, so they stay live without re-projecting.
- The sun-shadow block follows (M22, 208 bytes, `shared::GpuSunShadow`): three cascades of 64 bytes each (the world-to-cascade rows `u`, `v` and `depth` as `vec4`s, the texel size in meters and the depth bias in depth units), then the cascade count (0 without a shadowed sun) and flags (bit 0: the cascades hold moving casters only). The light records start at byte 400.

**`dynamic_triangles` (u32 words).** Henry's format, unchanged (`C/Core/DynamicTrianglesBuilder.cs:198-366`):

- **Per-triangle header:** `[lightDataOffset, boundsX−2, boundsY−2, W]`.
- **Light block:** `[count, (lightIndex, shadowOffset[, bounceOffset])…]`.
- **Shadow bits:** LSB-first, bit index `x + y·W`. Area lights and suns with a disk store 4 bits per texel and baked skies 8 (§12, Beyond Henry): `32 / bits` texels per word, texel `j` at bit `j · bits`; 1 bit is Henry's layout.
- **Bounce words:** packed at 4–8 bits per pixel.

Two changes for Bevy:

- Henry has one buffer per renderer. We concatenate all baked meshes into one `ShaderBuffer`, and the shader adds the receiver's base offset. Henry's offsets stay relative, so "0 = no data" still works.
- `triangle_index_submesh_offset` disappears. Each glTF primitive is its own Bevy mesh entity, so the primitive index starts at 0 per draw.

**Receiver records** (per mesh instance, found through `MeshTag`): `{triangles_base, lightmap_resolution (0 = not baked → dynamic-geometry path), flags}`.

- Bevy 0.20's `GpuComponentArrayBuffer` can manage the tags.
- Otherwise we assign stable tags ourselves. This fixes bevyout's `Added<Mesh3d>`-order counter, `viewer/scene.rs:83-86`.

**Aux buffer (u32).**

- Receiver records.
- Light BVH nodes: 32 bytes, `{aabb_min, left_first, aabb_max, count}`.
- Distance cubes: 64²×6 f16 pairs per baked light (12,288 words each), suns excepted.
- Sun maps (M19): after the distance cubes, a 16-word record per baked sun (three world-to-map rows, the resolution, the first texel word), then the depth texels, unorm16, two per word. Header words 8 and 9 hold their offset and count; the header is 12 words.
- Sky probes (M21): after the sun maps, an 8-word record (the grid's origin, spacing, three probe counts and the first probe word), then two words per probe, x fastest: the openness of the hemispheres around +X, −X, +Y, −Y, +Z and −Z as unorm8. Header word 10 holds the record's offset, 0 without probes.
- These are merged into one buffer to stay under the per-stage storage-buffer limit.

**Textures.**

- **Distance-cube index lookup:** `texture_cube<u32>`, R32Uint, read with `textureGather`.
- **Cookies:** 2D array.
- **Sky (M20):** the prefiltered specular cube, `Rgba16Float`, 128² with 6 levels, on `DynLitMaterial` at bindings 30–31 (`environment::SKY_SPECULAR`, a 1×1 black cube without a sky). The background pass has its own bind group with the equirect image (`SKY_BACKGROUND`, `Rgba16Float` with mips). The material bind group now holds 10 of WebGPU's 16 sampled textures and 10 of its 16 samplers per stage; storage buffers are unchanged.
- **Sun cascades (M22):** a `texture_2d_array` of three 1024² layers in the cube array's format, on `DynLitMaterial` at binding 32 and sampled with the cubemaps' sampler at 26 (`realtime_shadows::SUN_CASCADES`, 1×1 layers until a sun has realtime shadows). The material bind group now holds 11 of the 16 sampled textures; samplers stay at 10.
- **Realtime shadow cubemaps (VSM):**
  - Henry uses a 512² cube array with 16 cubes.
  - `Rg32Float` isn't filterable without the `FLOAT32_FILTERABLE` feature. **Done:** `Rg32Float` when the device has the feature (Bevy requests all adapter features by default), `Rg16Float` otherwise; the render world builds pipelines for both. Variance needs the precision: `Rg16Float` shows acne on caps and crate faces.

**Storage-buffer budget.** We use 4 storage buffers: lights, triangles, aux and the realtime light clusters (M17). With Bevy's view group (bindings 8, 9 and 10) and the mesh group (one), the fragment stage binds 8, exactly WebGPU's default per stage. Bevy asks for the adapter's limits, which are often higher (16 in Edge on an RTX 4080), so `gpu_check --webgpu-limits` caps the device at WebGPU's defaults, and a GPU test runs it (with the cap at 7, wgpu reports a count of 8). Another buffer needs triangles merged into aux, or raised limits on native. Triangle data with bounce can exceed the default 128 MiB binding size, so on native, raise `max_storage_buffer_binding_size`.

## 6. Primitive index and platforms

Henry's runtime keys everything on `SV_PrimitiveID`, which is WGSL `@builtin(primitive_index)`.

- **Requirements:** it needs wgpu's primitive-index feature, requested in `WgpuSettings`, plus `enable primitive_index;` in the root WESL module.
  - bevyout already proved this works in Bevy 0.19 with `MeshTag` + `primitive_index` + `uv_b`.
- **Availability:** desktop Vulkan, DX12 and Metal.
- **Fallback (done in M9):** for adapters without the feature (WebGPU, some mobile), a "triangle-ID attribute" mode.
  - Browsers always take it: wgpu 30's WebGPU backend doesn't map the `primitive-index` feature, even where the browser's adapter offers it (Edge 155).
  - Baked meshes are de-indexed once their `ResolvedBakeId` is known, and get a flat `u32` triangle-ID vertex attribute (`baked_lighting::triangle_ids`).
  - `DynLitMaterial::specialize` keeps its own vertex shader (`dynlit_vertex.wesl`) only for meshes with the attribute; every other mesh gets Bevy's.
  - The mesh content hash covers triangle corners in draw order, not the index buffer, so a de-indexed mesh still matches its bake.
  - `DynamicLightingSettings::triangle_ids` (`Auto` / `VertexAttribute`) forces the fallback, e.g. to test it on desktop (`lighting_test --triangle-ids`).
- **Startup check:** detect the feature at startup and pick a mode. Log clearly when baked lighting is unavailable.
- **Mesh stability:** the bake must see the same index order as the draw. That means no runtime re-merging or meshlets on baked meshes. Store a mesh content hash and detect mismatches.

## 7. Shaders: HLSL → WESL

### 7.1 Module map

| WESL module | From | Work |
|---|---|---|
| `dynlight::types` | `S/DynamicLighting.cginc` structs, flag getters, constants | Straight port. Merge with `BO/shaders/dynamic_lighting_types.wgsl` and drop its `PREPARED_INVERSE_SQUARE` branch |
| `dynlight::bindings` | Henry's globals | New: all resources on the material bind group (`@group(constants::MATERIAL_BIND_GROUP)`) at a fixed high binding range, so user materials can share them |
| `dynlight::math` | `S/Internal/Common.cginc` minus the BRDF and `rand` | Port: out-params → returned structs, overloads renamed, ternaries → `select` |
| `dynlight::light_types` | spot, discoball, wave, interference, rotor, shock, disco, water/random shimmer (+ trilinear wrappers from `S/Generators/TrilinearFilter.cginc`), attenuation, cookie UV | Port. `BO/shaders/dynamic_lighting_spatial.wgsl` covers most of it; fix its spot-direction sign |
| `dynlight::triangles` | `DynamicTriangle` load / set-active-light, bit read, shadow tiers integrated/low/medium/high, bounce decode | Mechanical port with explicit u32/i32 casts |
| `dynlight::distance_cubes` | `sample_distance_cube*`, angular filter | Port. Re-derive face order and lookup for right-handed cubes |
| `dynlight::vsm` | VSM test from `S/Generators/LightProcessor.cginc:122-137` | Port + NaN guard on the variance |
| `realtime_shadows/shaders/sun_vsm.wesl` | New (M22): the same test on the sun's cascades | Cascade choice and blend, the moments' Chebyshev bound |
| `dynlight::light_processor` | `S/Generators/LightProcessor.cginc` (pasted textually into each material in Unity) | Becomes `fn process_light(...) -> LightSample` |
| `dynlight::brdf` | Metallic GGX | Rewrite from published formulas (§2) |
| `dynlight::lighting` | `DYNLIT_FRAGMENT_*` macros, `S/DynamicLighting.cginc:1008-1174` | Restructure as functions: baked-triangle path, light-BVH path (u32 index stack), brute-force path, then the realtime loop. Lambert or PBR via `@if(DL_PBR)` |
| `materials/diffuse.wesl`, `metallic.wesl`, `unlit.wesl`, `prepass.wesl` | `S/DynamicLighting{Diffuse,Metallic,Unlit}.shader` | New glue on Bevy's default mesh vertex output (`uv_b`, color, tangent, instance index). Transparent = Diffuse + `AlphaMode::Blend` (Henry marks his Transparent shader deprecated) |
| `post/volumetric_fog.wesl` | `S/DynamicLightingPostProcessing.shader` | Migrate `BO/shaders/dynamic_lighting_volumetric.wgsl`. Rewrite depth reconstruction for reverse-Z / infinite far plane. Use its own light buffer instead of swapping the main one |
| `shadow/vsm_moments.wesl`, `shadow/gaussian_blur.wesl` | `S/Operations/ShadowDepth.shader`, `GaussianBlur.shader` | Port; new pass setup |

### 7.2 Translation rules (from the full read of `S/`)

- **Struct methods** become free functions taking the struct or a `ptr`.
- **Increments and decrements:** `x++` / `--x` inside expressions become separate statements, e.g. `stack[sp++]`, `tValues[n++]`, `offset++`.
- **Implicit conversions must be explicit:**
  - uint→bool (`!= 0u`)
  - bool→float (`select(0.0, 1.0, c)`)
  - float2→uint2 (`vec2<u32>(floor(p))`)
  - float4→float truncations
  - mixed int/uint arithmetic
- **Shifts:** `1 << i % 32` becomes `1u << (i % 32u)`. Wrapping u32 math is intended (`boundsX − 2` can wrap).
- **Local constant arrays:** `const float w[5][5]` and the face tables indexed dynamically become module-level `const` arrays or separable weights.
- **BVH stack:** the 32-node stack holding whole structs becomes `array<u32, 32>`.
- **`mul(v, look_at_matrix(f, u))`** is written as `vec3(dot(v, r), dot(v, u), dot(v, f))`. HLSL row-major vs WGSL column-major makes a literal copy transpose.
- **`switch`** needs `default:` and comma cases instead of fall-through.
- **Texture calls:**
  - `TextureCubeArray.SampleLevel(float4(dir, i))` becomes `textureSampleLevel(t, s, dir, i32(i), 0.0)`.
  - `GatherRed(..).wzxy` becomes `textureGather(0, t, s, dir).wzxy` on a `u32` cube.
- **Function and type renames:**
  - `f16tof32` becomes `unpack2x16float`.
  - `frac` becomes `fract`, `lerp` becomes `mix`.
  - `fixed`/`half` become `f32`.
- **Reserved words:** `common` is reserved, and parameters named `min`/`max` shadow built-ins; rename both.
- **NaN guards:** add them where HLSL `max()` hid NaN (VSM variance) and for the sky depth in fog (reverse-Z depth 0).
- **Handedness:**
  - Spot `forward` is uploaded as `transform.back()` (Henry negates forward).
  - Negate `right` for cookies, rotor and disco to keep Henry's look.

### 7.3 Keywords → `@if` flags and dropped Unity passes

**Flags:**

- Quality tiers: `DL_Q_INTEGRATED`, `DL_Q_LOW`, `DL_Q_HIGH`; Medium is the default.
- `DL_BVH`.
- Bounce, split in two:
  - `DL_BOUNCE_LAYOUT` sets the 3-word light-entry stride.
  - `DL_BOUNCE` enables sampling, and is set only when bounce is used and quality isn't integrated.
- Dynamic geometry: `DL_DYNGEO_CUBES`, `DL_DYNGEO_ANGULAR`.
- Culling: `DL_CULL_FRONT`, `DL_CULL_OFF`.
- Material features: `DL_ALPHA_MASK`, `DL_ALPHA_PREMUL`, `DL_EMISSION`, `DL_METALLIC_TEXTURE`, `DL_PBR`.
- Debug: `DL_DEBUG_LIGHTING_ONLY`.

These flags are scene-level settings, so changing them re-specializes pipelines. That's fine, because they change rarely.

**How the scene-level switches are compiled (2026-10-17).** Henry compiles one shader variant per keyword combination. Bevy's shader defs are per pipeline key, so a scene-wide switch would have to be copied into every material key. Instead, `shared/variant.rs` generates a WESL module, `dynlight::shared::shaders::variant`, holding `const QUALITY: u32` and one `const FEATURE_*: bool` per entry of `DynamicLightingSettings::shader_features`. The module is an asset under a fixed handle. When the settings change it is replaced, and Bevy's shader cache recompiles every pipeline that imports it. The lit shader branches on the constants (`if QUALITY >= 2u`, `if FEATURE_AREA_LIGHTS && …`), and the backend compilers drop the dead branches as they would `if false`. Per-material switches stay in the material key: `METALLIC_SHADING` (from `ShadingModel::Metallic`) and Bevy's `MAY_DISCARD`.

- The features are `light_patterns` (spot, disco, wave, … masks and cookies), `area_lights`, `sun` (M19), `sky` (M20; baked and realtime skies alike), `realtime_lights`, `realtime_shadows` (the cubes and, from M22, the sun's cascades), `unbaked_meshes` and `bounce` (the applied bake has bounce data). All are on by default. With one off, its lights or meshes render as if that path weren't there: area lights as points, patterned lights without their mask.
- `ShaderFeaturesInUse` collects what the scene uses each frame (the light slice from the packed lights, `surface` from meshes without a `BakedReceiver`). When a feature is in use for 3 s while switched off, a warning names it once.
- **Automatic selection (2026-10-18):** `DynamicLightingSettings::feature_selection = FeatureSelection::Automatic` compiles only the features of `shader_features` the scene uses, the way Henry's manager sets its keywords at scene load. Default stays `Fixed`.
  - Until the first choice there is no variant module, so lit pipelines wait for it instead of compiling twice.
  - Slices set `SceneLoading` while data that decides the use is pending: `baked_lighting` while `ActiveBake` names a bake that hasn't loaded or failed, `bake` while a bake runs. The choice is made once the scene has not been loading for 5 frames (`shared/feature_choice.rs`), or after 600 frames regardless. The scene's use in that last frame is what gets compiled.
  - A feature first used later is added at once, which recompiles: lit meshes vanish for the compile, because Bevy re-queues every pipeline whose shader import changed. When the scene loads again (another bake), the features stay until it settles, and are then chosen afresh, possibly fewer.
  - Henry never strips light types, because Unity has every variant precompiled and switching is free; in Bevy a switch costs a compile. So this is opt-in, for scenes whose kinds of lights and meshes are known when they load. The result is `LitShaderFeatures`.
  - Checked: the harness's baked scene chooses no features at all (the `--lean` variant) with an identical image, the realtime and shadow scenes keep their paths with identical images, and `lighting_test` uses every path and compiles once. The baseline `lighting_test_automatic_features_match_the_overview` guards it.
- `GpuLightGlobals::quality` stays in the buffer for readback checks; the shaders read `QUALITY`.

**Dropped:** ForwardAdd passes, the ShadowCaster pass, Unity lightmaps (`LIGHTMAP_ON`), reflection-probe macros, Unity fog, instancing macros, the legacy transparent shader, and BlitColorDepth.

**Replaced:**

- `Material::enable_shadows() = false`.
- An alpha-cutoff prepass fragment shader.
- An optional own `texture_cube` reflection map for Metallic. Never `EnvironmentMapLight`.
- The sky (M20): our own sky light and background pass (`environment/`), never `EnvironmentMapLight`, `GeneratedEnvironmentMapLight`, `Skybox` or `Atmosphere`. A material's own reflection map takes the place of the sky's reflection.

## 8. Baker (`bake` slice), a port of `C/Core/DynamicLightingTracer*.cs`

**Input.** `BakeScene` is built from the ECS:

- Static meshes in world space (main-world `Mesh` data: positions, normals, indices, UV1 with a UV0 fallback, optional UV0 + base-color alpha for transparency), each with its `BakeId` and content hash.
- Baked lights, with their `BakeId`s.

**Steps** (Henry's order, with replacements marked ⇒):

1. **Light BVH.** Port `BvhLightStructure` (SAH, leaf ≤ 2), then reorder lights into BVH order. Skip channel assignment: Henry's runtime never reads channels; they're bake-time scratch.
2. **Mesh gathering + MeshBuilder.**
   - Lightmap size = `ceil(sqrt(density² · area))`, clamped to the maximum (2048).
   - ⇒ Add a lower bound of 2 to avoid Henry's size-1 out-of-bounds risk.
3. **Occluder BVH** over all `BakeStatic` triangles ⇒ `obvhs` (or the `bvh` + `rayon` tracer from `bevyout/src/vsa/prepare/static_shadows.rs:449-600`) behind a `RayCaster` trait. Two-sided hits, as Unity's `queriesHitBackfaces` gives.
4. **DynamicTrianglesBuilder.**
   - UV bounds with ±2 px padding.
   - Light association: BVH query, closest-point-on-triangle within the radius, and the NdotL > −0.1 face test.
5. **RaycastTriangle**, per texel and light.
   - Port `TriangleUvTo3dStep`: texel center UV → barycentrics → world position and normal. ⇒ Use an explicit "outside" flag instead of Henry's (0,0,0) sentinel, and renormalize interpolated normals.
   - Range and NdotL early-outs.
   - Ray from `world + n·0.001` to the light; a miss sets the bit. ⇒ Parallelize with `rayon` over (mesh, triangle) instead of RaycastCommand batches.
   - ⇒ Area lights and suns with a disk store the lit share of their shape in 4 bits (M14, M19), and a sky the open share of the hemisphere in 8 bits from `BakeSettings::sky_samples` cosine-weighted rays (M21, `bake::sky`). §12, Beyond Henry, has the details.
6. **Transparency modes.** Henry samples GPU photon cubes. ⇒ Our tracer alpha-tests base-color alpha (UV0, cutoff 0.5) directly during any-hit traversal. This is simpler and more accurate. `EnabledMax` maps to the same thing.
7. **Bounce** (SingleBounce lights).
   - Henry's algorithm: Slerp spread 0.1→1.0 from light→receiver toward a random direction; photon lookup; visibility ray; `NdotL·NdotL_photon` accumulation; dilate; blur (kernel 7, σ 5); sqrt + quantize with dither.
   - ⇒ Build photon cubes with CPU closest-hit rays from the light: position + normal per texel, default 512² per face (Henry: 1024²; configurable).
   - ⇒ Seed the RNG per (mesh, light, texel, sample).
8. **OptimizeTriangle.** Drop lights whose bits are all zero in the triangle rect, unless they have bounce data.
9. **Seam padding + DilateShadowBits.** Port. ⇒ Fix the x−1 row wrap at x = 0.
10. **Serialize** the per-mesh `dynamic_triangles` words exactly as Henry does.
11. **Distance cubes.**
    - Per baked light, 64² per face, via CPU closest-hit rays.
    - The `DistanceCubes` mode adds a +0.25 bias; `Angular` doesn't.
    - Build the R32Uint index-lookup cube for Bevy's face convention.
    - ⇒ Distant lights have no position for a cube: each baked sun gets a sun map (M19, `bake::sun_map`), and a scene with a baked sky one grid of sky probes (M21, `bake::sky_probes`).
12. **Write the `.dynlight` asset.**
    - Container: lift bevyout's codec (magic, version, gzip via `flate2`, a CRC-32 checksum via `crc32fast`).
    - Sections: settings (bounce on, dynamic-geometry mode), lights (`BakeId`, baked origin, compression, shape, bits per texel; local lights, then suns, then skies), BVH nodes, meshes (`BakeId`, content hash, resolution, triangle words), distance cubes, sun maps, sky probes, stats. The layout is in `bake/format/mod.rs`; version 4 since M21.

**Not reused from `BO/baker.rs`:** it uses one centroid ray per triangle, a brute-force tracer, placeholder bounce, dominant-plane UV1, and silent truncation to 32 lights.

**Requirements and options:**

- UV1 is required, as in Henry. Missing UV1 falls back to UV0, with a warning.
- Automatic UV1 generation via `xatlas` (MIT) is an optional later step.

**Orchestration.**

- A `BakeRequest` message snapshots the ECS and runs on `AsyncComputeTaskPool`, with progress and cancel.
- Output is always an in-memory `DynLightBake` asset returned as `BakeReport::handle`; `BakeRequest::output = Some(path)` additionally writes `assets/<scene>.dynlight` on native. `None` avoids the filesystem and supports browsers. Saved assets can still be loaded/hot-reloaded.
- Presets: Preview (no bounce) and Full, with a resolution choice like Henry's menu.
- A `--bake` flag on the examples bakes at startup and applies the result; with `--frames` the example then exits, so CI can bake headlessly once runners have a GPU.

## 9. Runtime manager (port of `C/Core/DynamicLightManager*.cs` as Bevy systems)

Per frame, in `PostUpdate` after transform propagation:

1. **Clock and effects.** Use a time resource and per-light `FixedTimestep` (lift `BO/core/fixed_timestep.rs`).
   - Henry's 15 effects come from the lifted `BO/core/runtime.rs` with our noise and RNG.
   - Flicker, Random and Strobe step on the fixed timestep; the others are continuous.
   - ⇒ Fix Henry's double update of moved baked lights.
2. **Light positions.** Live tracking (every frame) or Relaxed tracking (on change), from `GlobalTransform`.
3. **Moved baked lights.** A baked light whose position differs from its baked origin gets `radius_sqr = −1` in its baked slot and is moved to the realtime list.
4. **Realtime selection.** Budget 32. If over budget, sort by distance to the active camera, then frustum-test the light spheres (our own sphere-vs-`Frustum` test; the camera frustum isn't lighting). Then `light_clusters` splits that camera's view into cells, each listing the selected lights that reach it, so a fragment visits only its cell's lights (M17, §12 Beyond Henry).
5. **Pack lights.** Fill `GpuLight`s (packing from `BO/render/gpu.rs`, `gpN` per type, gamma→linear colors, bounce color mix, `falloff = r·falloff²`). Write the `ShaderBuffer` (resizable without rebuilding materials).
6. **Cookies.** Dedupe per texture into the cookie array, budget 64. Copy only when a cookie changes; Henry re-blits every frame.
7. **Realtime VSM shadows** (§10) for `RealtimeShadows` lights within the budget of 16.
8. **Volumetric lights.** Build a separate persistent buffer for the fog pass, without the cap of 2 that bevyout had.
9. **Upload once.** Load-time data (triangles, aux, lookup cube) is uploaded once when the asset loads. ⇒ Fixes bevyout re-uploading the whole bake every frame (`BO/render/gpu.rs:302-334`).
10. **Stale detection.** Mesh or light hash mismatches against the bake fall back to the dynamic path and are reported in diagnostics.

## 10. Render passes (our own, as systems in `Core3d`)

**Realtime VSM shadow cubemaps** (Henry's `DynamicLightManager.ShadowCamera.cs`). Per light: 6 faces at 512², storing moments `(d/r, (d/r)²)`, then a separable Gaussian blur (kernel 3, σ 1) into the cube array.

- **Spike A (M8–M16, replaced in M18):** Bevy 0.20's `render_depth_to_texture` example. Depth-only cameras (`RenderTarget::None` + `DepthPrepass`, 90° FOV, one per face), then our own system after `Core3dSystems::Prepass` converted depth to linear-distance moments and blurred them. M16 dropped the `Clusters` component Bevy requires on every `Camera3d`. Each face still cost a camera's CPU work: visibility over every entity, camera extraction, prepass textures, phase extraction.
- **Spike B (M18):** faces are views of their own drawn by our own render phase, the pattern of Bevy's point-light shadows without its types (`ShadowView`, `Shadow`).
  - Face entities (`realtime_shadows::views`) are synced to the render world. A face rendering this frame gets an `ExtractedView` with the same face matrices, `NoIndirectDrawing` (Bevy builds indirect parameters only for prepass views and its own `ShadowView`) and its caster list (`extract`). Faces that don't render lose them, so nothing runs for them.
  - One driver renders every face and cascade view (`driver`, M24). It is a render-world entity with `RootNonCameraView(ShadowViews)`, which Bevy's camera driver runs before the cameras. Its schedule (single-threaded, its systems' command buffers applied when it ends) first does Bevy's direct mesh preprocessing for every view in one compute pass, with Bevy's own pipeline, bind groups and work items, then draws every view into one command encoder (`render::shadow_face_pass`). Until M24 each face was a `RootNonCameraView(Core3d)` of its own: a full `Core3d` schedule run per face, with four command encoders and two empty compute passes (Bevy's uniform allocation and bin unpacking, which have no work for direct views). A view whose preprocessing didn't run (pipeline compiling, bind groups missing) is reported and renders again, since it would draw last frame's mesh transforms.
  - Casters come from our own culling (`casters`): the bounding spheres `tracking` keeps, tested against each face, with the camera path's rules (default render layer, hidden meshes left out, meshes without bounds in every face, baked meshes left out of cubes whose light stands where it was baked). Listed casters are marked visible in `CheckVisibility`, so Bevy extracts them when no camera sees them.
  - The binned phase `ShadowFace3d` (`phase`) is drawn with Bevy's `DrawPrepass` and `DrawDepthOnlyPrepass`. Pipelines come from each material's `prepass_specialize` with the key Bevy's depth prepass uses (`queue`): opaque and masked materials cast, blended ones don't. Bevy's GPU mesh preprocessing serves the face views in direct mode.
  - The pass draws the casters into one shared 512² depth texture, then runs the moments and blur passes.
  - Readiness is per face: a caster whose mesh or material isn't prepared, or a draw that skips (pipeline compiling, Bevy's preprocessing not ready), reports its face (`phase::Reported`), and only that face renders again next frame. The camera path redrew every cube while any pipeline anywhere compiled.
- **The sun's cascades (M22)** are three more views of the same kind (`realtime_shadows::sun`): orthographic, reverse Z, 1024², with their own depth and scratch textures (`targets`, made once a sun has realtime shadows) and the same phase, queue, moments and blur passes. The face uniform's `orthographic` flag makes the moments pass store the linear depth along the sun's rays (`1 − depth`) without the cube faces' mirror. Readiness is reported per cascade like per face (`tracking::ShadowTarget`).
- **Not used:** bevyout's `shadow_proxy.rs`, its patched `bevy_pbr` cubemaps, and its deferred surface pass. That surface pass was dead code, because the app runs forward and never adds `DeferredPrepass`.

**Volumetric fog.** Migrate the existing pass (`BO/render/pipeline.rs:149-181,300-362`); it is already a `Core3d` system.

- `EarlyPostProcess` → `Core3dSystems::PostProcess`. **Done differently:** it runs after tonemapping, like Henry's, which composites fog over the finished image; before tonemapping the fog's premultiplied lerp would be tonemapped too and look different.
- Replace its screen blend on HDR values.
- Use the persistent volumetric buffer.

## 11. What happens to bevyout code

| Action | Files |
|---|---|
| **Lift into slices** (see §3; split anything over 400 lines) | `core/{config,types,fixed_timestep,spatial,volumetric,runtime}.rs`; `unity_math.rs:18-46` helpers (renamed); GPU light ABI + packing + tests from `render/gpu.rs`; codec + quantizer from `baker.rs` (`pack_quantized` 533-553, codec 123-183/555-714, codec tests) |
| **Migrate** | `bevy_bridge/mod.rs` (component/runtime/settings/diagnostics/tests; drop `DynamicLightingBakeRuntime` form IDs); `render/mod.rs` and `extract.rs` (drop proxy and prepared plumbing); `shaders/{types,common,spatial,volumetric}.wgsl` → `.wesl` (`#define_import_path` and `#import` → `import …;`, no `#ifdef`s present); `viewer/lighting_test.rs` → `examples/lighting_test.rs` (effect / type / fog grids + capture state machine); `tests/dynamic_lighting_gpu.rs` screenshot harness (add value checks vs CPU mirrors); `features/dynamic_lighting.feature` scenarios 1–10 |
| **Rewrite** | Baker tracing (§8); `viewer/scene.rs` artifact loading → `AssetLoader`, stable tags; forward-hook logic from the patched `pbr_fragment.wgsl:44-158` → `DynLitMaterial` (drop the magic `*0.01`, `*0.5`, `*0.08`) |
| **Drop** | `core/unity_random.rs`; Perlin in `unity_math.rs`; `reference_unity/*`; `tests/golden/*` (~26k lines); Unity-parity tests (`runtime.rs:297-401`, `spatial.rs:216-240`, `volumetric.rs:431-507`, `config.rs:177-178`, `tests/features.rs` UnityRandom steps); `bevy_bridge/shadow_proxy.rs`; `render/pipeline.rs` surface pass; `shaders/dynamic_lighting_{surface,pass}.wgsl`; `third_party/bevy_pbr-0.19.0` + `[patch.crates-io]`; `viewer/{lighting,lighting_policy,material_shading,…}.rs`; `features/hybrid_lighting.feature`; scenario 11; all Fallout coupling (form IDs, manifests, GLB extras key `bevyout_dynamic_lighting_mesh_tag`, CLI) |

## 12. Quirks in Henry's code: fix or keep

**Fix:**

- Metallic applies intensity twice (`S/DynamicLighting.cginc:335` + `Metallic.shader:298,300`).
- Metallic bounce cross-terms (`Metallic.shader:298,340-341`).
- `set_active_light_index` keeps stale shadow and bounce offsets when it switches to a realtime light (`S/DynamicLighting.cginc:685`).
- Seam padding wraps across rows.
- Interpolated normals are not renormalized.
- Texels at world (0,0,0) are treated as outside.
- The double effect update for moved baked lights.
- The unnormalized reflection normal.
- The documented-but-missing realtime-light fade: implement it as a short fade, or drop it from the docs.
- Spot and discoball lights with equal inner and outer cutoffs divide by zero in the cone falloff (`S/DynamicLighting.cginc:144,185`). We give them a hard edge (`light::mirror`; the WESL must match).
- Realtime shadow cubemaps hold 16-bit moments on GPUs without filterable 32-bit floats; Henry always uses `RGFloat`. We use `Rg32Float` wherever possible.
- The VSM variance can round below zero in half floats and give NaN; it is clamped to 0.
- Jagged realtime shadow terminators. Henry's light-bleeding cut `max(0.3, 1 − NdotL, depth)` takes the half-Lambert NdotL of dynamic geometry. Together with the 0.75% depth bias, it lets wrap light reach about 35° past the terminator of a curved surface. Then the 512² map cuts the light off along a jagged line of texels: saw-tooth bands down pillars, crescents on cylinder caps, dark outlines along crate edges. We drop the `1 − NdotL` term. Instead, a shadow-casting light fades with a smoothstep of the true cosine, from −0.1 to 0.3 (`realtime_shadows::mirror::terminator`). For an area light that is the cosine to the point of its shape the surface faces most (`light::area::facing_cosine`), not to its center, which is only where the cubemap is seen from. Surfaces turned away from a shadow-casting light lose its wrap light; lights without a cubemap keep it. Normal-offset lookups were tried as well: they only moved cast-shadow edges and did nothing for the terminator.
- Stair-stepped cast-shadow edges. The 3-tap σ = 1 face blur leaves the 512² texels visible along shadow edges, so we use 5 taps at σ = 1.5. Shadows are slightly softer. Area lights blur wider (Beyond Henry).
- Realtime shadows switch off for good once a frame fills every shadow cubemap. Henry resets the cubemap index only when a frame draws its first cubemap (`C/Core/DynamicLightManager.ShadowCamera.cs:129`), and that happens after the budget check (`:164`). After a frame that used all 16 cubemaps, the index stays at 16, so every later frame skips all realtime shadows. It still happens on his master as of 2026-10-17, and the Unity comparison (§14) found it. We pick the shadowed lights again every frame, nearest the camera first, up to the budget. `CubeSlots::assign` keeps a light's slot while it stays picked and frees the rest, so a full budget only leaves the farthest lights without a cubemap.
- The High shadow filter reads three texels past a triangle's grid while the padding is two. Near the top of the grid its index wraps below zero and reads outside the triangle's data (Henry's `uint` math; the value is whatever the GPU's robust buffer access returns). We read the grid's first row there, and texels before its first texel read that texel. Everywhere else the filters read the same bits as before: `triangles.wesl` now reads eight bits of a row with at most two loads (High: 16 loads per light instead of 400, Medium 8 instead of 16) and applies the 5×5 Gaussian across then down. `baked_lighting::mirror` checks the row reads against texel-by-texel reads, and the lab renders pixel-identical at every tier.

**Keep** (they define the look):

- The fog composite lerps towards the premultiplied fog color (`S/DynamicLightingPostProcessing.shader:179-183`), so thin fog lifts dark pixels but slightly dims bright ones. `fog::mirror::compose` and its test pin this.

- The −0.1 NdotL tolerance.
- The ±2 px padding.
- The 0.001 ray offset.
- Attenuation `intensity·(1−s)²/(1+falloff·s)`.
- The shadow filter kernels.
- Bounce sqrt-encoding.
- The effect formulas. Thresholds are re-calibrated for our noise.
- The VSM test: 0.75% relative depth bias and the light-bleeding cut `max(0.3, depth)` (Henry's, minus the `1 − NdotL` term; see Fix).

**Also fixed** (found while writing `docs/PORT_MAP.md`):

- Bounce blur passes read from a copy; Henry's read and write the same texture, which smears values along the pass direction.
- `ApproximatelyEquals` compares relative to the magnitude (`bake/geometry.rs`); Henry's compared absolute values.
- Trace and optimize rectangles come from texel centers; Henry scales UVs by size − 1 and `OptimizeTriangle` uses an exclusive maximum.
- Bounce samples are clamped to at least 2; Henry divides by samples − 1.
- Cookies: spot cones over 90° or at 0° get no cookie (Henry's guard), and array layers of images no light uses any more are reused once the budget is full.

**Different by design** (not bugs in Henry's code, but Unity behaviour we can't or won't reproduce):

- Bake identity: `BakeId` comes from the entity's `Name` path (collisions get `#k` suffixes, ordered by content hash and position) instead of Unity instance IDs. The runtime stores it as `ResolvedBakeId`.
- Shadow cube faces are views of our own render phase, drawn with Bevy's depth-prepass pipelines (§10); the moments and the horizontal blur run in one pass, mirrored into the left-handed cube face, then a vertical blur pass writes the array layer.
- Bounce light needs non-overlapping UV1 (one bounce map per mesh, as in Henry's). Bevy's `Cuboid` UV0 overlaps, so the examples build a UV1 atlas.
- `Transparency::EnabledMax` behaves like `Enabled`: our shadow rays alpha-test directly, so there is no photon-cube alternative.
- Directional lights don't exist in Henry's runtime; his editor menu makes a point light 2.5 km away with a 10 km radius. `light::sun_light` makes a `LightType::Sun` instead (Beyond Henry).
- Metallic shading: diffuse isn't divided by π (the specular is scaled by π instead), so a rough dielectric under Metallic shading matches Diffuse shading; specular uses plain Lambert on dynamic geometry.
- Moved baked lights: a light counts as moved beyond 1 mm from its baked origin (Henry: 1e-5 m). A radius grown more than 1 mm past the baked one counts as moved too (Henry doesn't check the radius): the triangle light lists, the light BVH and the distance cube end at the baked radius, so the light would stop there on baked surfaces and moving objects past it would read the cube's empty-space distance as a blocker. A smaller radius keeps the bake.
- Shadow cube faces end at the light radius (Henry: 3× the radius). Occluders beyond the radius can't stand between the light and anything it reaches. Cubemaps go to the lights nearest the camera first; Henry hands them out in light-buffer order.
- Transparency: shadow rays alpha-test the texture directly, with the material's own mask cutoff, so Henry's 5×5 alpha filter over photon cubes has no counterpart.
- The light BVH query also tests each light's own sphere, so leaves never return lights out of range.
- Bounce sampling seeds one random stream per texel (not one per sample); the output is still deterministic.
- `TrackingMode` has no effect: transforms are change-tracked in Bevy, so both modes see current positions.

**Beyond Henry** (features his package doesn't have, M10, M11 and later, and their approximations):

- Shadow cubemaps keep their slot and only redraw faces that changed (`realtime_shadows::slots`): a face renders again when its light moves or changes radius, the array is recreated, the set of casters it draws changes, a `Mesh` or `DynLitMaterial` asset changes, or a caster that moved, appeared, disappeared or is skinned overlaps the face's frustum. A caster has moved only when its world transform differs from the last one seen: Bevy marks a transform changed whenever a system writes it, and game code often sets one every frame while the object stands still. A face also redraws when one of its casters wasn't drawn (its mesh, material or pipeline not ready yet, M18). A light standing where it was baked leaves baked meshes out of its cubemap (`realtime_shadows::casters`): their shadows are in the bake already, so its cubemap only holds moving casters.
- Area lights (`LightType::Rectangle`, `LightType::Tube`) are two more light types in Henry's family. Their shading comes from a shape, and their baked shadows break one of Henry's rules: a texel stores how much of the shape it sees, in 4 bits, instead of 1 bit for a single point.
  - Kept: shadows are traced once per texel at bake time over static geometry in a fixed triangle order; the light is one record at its center with Henry's `radius` sphere (light BVH bounds, per-triangle light lists, distance cube and stale detection as for a point light); a baked light that moves, turns, changes its shape or grows its radius falls back to realtime lighting until the next bake (`light::placement`); its color, intensity and effects can change freely (and a tube's unused height). Point and spot lights keep 1 bit and bake the same shadow bytes as before.
  - Baked visibility (`bake::area`): probe rays go to a rectangle's corners and center, or a tube's ends and center, then `BakeSettings::area_light_min_samples` rays (16 by default) go to the first points spread over the shape. When all of them agree, the texel is fully lit or fully shadowed. Otherwise the spread continues to `BakeSettings::area_light_samples` rays (64 by default; the lab uses 128). Preview bakes trust the probes alone and take at most 32 rays. The points (`bake::spread`) are the Sobol (0, 2) sequence over a rectangle and van der Corput's sequence along a tube, offset per texel by a stateless hash of the seed, mesh, light and texel. The first 2^k points put one point in each of 2^k equal strips along each axis, so a thin occluder (a pole, a railing) that hides 2/16 of the shape's width or height is always seen, even when it passes between all the probes; the probes alone missed it, and so did the first 16 points of the R2 sequence used before, whose spread along one axis clumps into four groups. Fully lit and fully shadowed texels cost 21 rays to a rectangle (5 before) and 19 to a tube (3 before); texels in a penumbra cost the same as before. Each ray is weighted by `(n·d + 0.1)` (Henry's −0.1 NdotL margin), the emitter's cosine and `1 / d²`, and the lit share is rounded to the nearest of 16 levels (`bake::shadow_grid`). Seams and padding treat levels like Henry's bits.
  - Data: shadow words pack `32 / bits` texels (§5); channel bits 20–22 carry the bits per texel. The `.dynlight` format version 2 stores each light's kind, rotation, size and bits per texel (60-byte light records) and rejects version 1. The shaders read a row of eight texels with at most two loads at any depth, so every filter tier costs the same loads as for 1 bit and returns levels over 15 (`baked_lighting::mirror` checks every tier against texel reads at 1, 2 and 4 bits).
  - Moving objects (both cube modes, `dynamic_geometry::mirror`): the distance cube stays at the light's center and now stores exact distances (Henry's +0.25 m `DistanceCubes` bias moved into the shader). A light point offset by `o` sees a blocker a fraction `s` of the way to the receiver where the center sees `d + o · (1 − s) / s`. A blocker search over the shape scaled by 3 finds the mean `s` (each blocker's distance over the receiver plane's along the same ray, exact for blockers parallel to the receiver), then 8 lattice taps over the shape scaled by `(1 − s) / s` give the share of the light that reaches the receiver. Every lookup blends four depth tests bilinearly (Henry's angular test), the filter's on grids of 1, 2, 4, … texel blocks sized to the taps' spacing and blended between the two nearest levels, so penumbras ramp without steps. Plane distances stop at the light's radius, which the cube stores for empty space. Limits: blockers nearer the light than a quarter of the way soften as if they were a quarter of the way; penumbra ends round off by about a tap's spacing (within 0.075 of a traced reference in the tests); a lookup costs 32 to 96 buffer reads per area light per fragment, only on meshes without bake data.
  - `radius` is the sphere around the light's position where the light ends, as for every type. Past its shape the light reaches `radius` minus half the shape's diagonal (`DynamicLight::shape_reach`, packed as the record's radius); a light whose shape doesn't fit inside its radius is invalid and disabled, like a broken spot cone. The bake skips texels farther than that from the shape.
  - Shading (the only new math): diffuse is the mean cosine over the shape (Lambert's polygon form factor for rectangles, the closed-form line integral for tubes, each over the angle the shape covers); a rectangle also has a Lambertian emitter cosine at its nearest point, so it lights only its front. Attenuation uses the distance to the nearest point over the reach past the shape.
  - Specular (Metallic shading, M15): a rectangle integrates the GGX lobe over its shape with linearly transformed cosines (Heitz, Dupuy, Hill and Neubelt 2016; `light::ltc`, `light/shaders/ltc.wesl`). The authors' fitted table (64 × 64 × 2 RGBA16F, BSD-2, `LICENSES/LTC-BSD-2.txt`) gives the inverse transform and the BRDF's albedo with and without Schlick's Fresnel factor by roughness and view cosine; the quad goes into the shading frame and through the transform, is clipped to the horizon, and Lambert's polygon formula (with the authors' rational fit of θ / sin θ) gives the lobe's share over it. Like the diffuse term it is divided by the solid angle (`π · albedo · share / Ω`), so the light's strength means the same for every shape and a small rectangle reflects like a point light. Tubes, and rectangles under 10⁻⁶ sr, keep one representative point with Karis's (2013) energy normalization per axis; diffuse light is still weighted by the Fresnel term at that point.
    - Accuracy: the polygon integral matches the transformed lobe integrated over the panel within 1%; the fit is within about 1.5% of brute-force GGX over panels inside the highlight and up to about 23% low in the lobe's tail (7% mean over the test cases, where the representative point was 15% off on average and up to 43%). At a pixel of `gpu_check_specular` under a 2.4 × 1.2 m panel 1.2 m above a roughness-0.3 floor, the representative point was 3.3× too bright (brute force 1.053, LTC 1.058, before 3.43); large panels close to shiny floors lose most of their old glare.
    - The fit's GGX uses Smith's height-correlated masking, so for the same light a small rectangle reflects 8–14% more than a point light, whose BRDF uses Schlick-GGX with Karis's `(r + 1)² / 8` (`surface::mirror` test). The jump at the 10⁻⁶ sr fallback is that difference, on a highlight smaller than a pixel.
    - Cost: two table samples and one more clipped quad per rectangle light per fragment, on Metallic materials only. The table is one more texture and sampler on the material bind group (bindings 27 and 28), uploaded once.
  - Realtime shadow cubemaps are seen from the light's position. Their faces are blurred wider the larger the shape is next to the radius: σ from 1.5 up to 4 texels, reached when half the diagonal is a quarter of the radius (`realtime_shadows::faces::face_blur`, 5 taps a side at σ 4). This softens the shadows of moving objects evenly; it doesn't model a penumbra that grows with the distance between occluder and receiver.
  - Why 4 bits: M12 and M13 kept Henry's 1-bit rule by baking an area light as 3 to 16 point lights. Penumbras came in visible steps (at most 16, wider than the filter tiers blur), and each point cost a light slot, a distance cube, per-triangle list entries and shadow reads. Soft visibility costs 4× a 1-bit light's shadow words, more bake rays only in penumbras, nothing extra at runtime, and gives smooth penumbras that harden where objects touch the ground. Lights on moving objects lose nothing: they never used baked shadows.
- Suns (`LightType::Sun = 10`, M19): parallel light from a disk at infinity, along the entity's forward axis, with `DynamicLight::angular_diameter` (0 to 10°, default 0.53°, the real sun's). They follow Henry's model of a light: color, intensity and effects change live, and the packed intensity carries the effect.
  - No position or radius: the shaders measure no distance (radiance is the intensity), and suns take no cookies, patterns, fog volumes or shadow cubemaps (one sun can have realtime cascades instead, M22, below). They are in neither the light BVH nor the cluster grid, and every lit fragment visits every sun. Diffuse light comes from the direction, Lambert on baked meshes and Henry's squared half-Lambert elsewhere; the disk would change it by less than 1%. Specular light (Metallic) comes from the direction within the disk nearest the mirror direction, with Karis's energy normalization `α′ = saturate(α + sin θ / 3)` (`light::sun`).
  - Kept: a sun with no disk bakes Henry's 1 bit per texel, from one ray with his 0.001 m offset and −0.1 NdotL tolerance; rays have no end. A baked sun keeps its bake wherever its entity stands, but turning it or changing its disk falls back to realtime lighting until the next bake (`light::placement`): its baked slot is disabled and it shines unshadowed from the realtime suns.
  - Bent: a sun with a disk bakes 4 bits of visibility, like an area light (`bake::sun`). Probe rays go to the disk's center and four rim points, then `area_light_min_samples` and up to `area_light_samples` rays go through the Sobol (0, 2) spread mapped onto the disk by Shirley and Chiu's concentric mapping (`bake::spread::concentric_disk`), offset per texel. Each ray is weighted by `(n·d + 0.1)`. Preview bakes trust the probes. The disk's frame comes from the direction alone (`any_orthonormal_pair`), so turning a sun about its own axis bakes the same bytes.
  - Slots: baked suns take the last baked slots, ordered by `BakeId`, and every triangle facing a sun lists it. The `.dynlight` format version 3 stores a sun as light kind 3 with its disk in the size and rejects suns before other lights.
  - Not baked: bounce light from suns. Henry's bounce traces photon cubes around a light's position; a sun would need a scene-wide photon map. Suns with `Illumination::SingleBounce` bake direct light only.
  - Moving objects (`dynamic_geometry::sun_mirror`, `shaders/sun_map.wesl`): each baked sun gets a sun map, the sun's counterpart of a distance cube. It is an orthographic depth map of the first static surface along the sun's rays (alpha-tested when the light's transparency is on), over the square bounds of the static triangles seen from the sun plus a depth margin of 1% + 5 cm, `BakeSettings::sun_map_resolution` texels a side (1024 by default, at most 512 in Preview bakes, 2 MB at 1024²), stored as unorm16 in the aux buffer. A lookup tests 4×4 texels weighted like the baked shadows' Medium and High filters (four overlapping 3×3 averages blended bilinearly), with a bias of 5 cm plus 2 texels; points off the map are lit. With the 2×2 bilinear test first tried, edges on surfaces the sun hits at a slant stepped visibly at each texel. Cost: 16 buffer reads per baked sun per fragment, only on meshes without bake data. Limit: it holds static geometry only, so moving objects receive sun shadows from it but cast none; M22's cascades (below) let them cast.
- Realtime sun shadows (M22, `realtime_shadows::{cascades, sun}`, `shaders/sun_vsm.wesl`): the brightest sun with `ShadowMode::Realtime` (ties by name path; with more than one, a warning names the chosen one once) gets three cascades over the view of the camera the realtime lights are chosen for, up to `DynamicLightingSettings::sun_shadow_distance` (50 m). Henry has no sun, so this is his cube-face VSM applied to the sun's rays.
  - Kept: his moments, the σ 1.5 blur, the Chebyshev bound with the 0.3 light-bleeding cut, and our smooth terminator (−0.1 to 0.3 of the true cosine). A sun standing where it was baked draws only meshes without bake data, like a cube of a light in place: the bake holds the rest, so moving objects cast shadows onto the baked scene and onto each other. On baked receivers of such cascades the terminator is left to the bake's Lambert term, since the map doesn't hold them. A sun that turned, or one moving meshes ignore the bake for (`LightingOnly`), draws everything.
  - Fit: the view splits at the practical split scheme's depths (λ 0.75; 0.1 m to 4.8 m, 13 m and 50 m with Bevy's 0.1 m near plane). Each cascade is a square around its slice's bounding sphere, with the half side rounded up to 1/64 m and wide enough that the sphere stays inside the cascade's 2% rim while the center snaps to a 64-texel grid in the sun's frame. Its depth runs from the sunward end of the casters over it to the far side of the sphere, snapped to the same step. So a cascade holds still while the camera moves less than a step, moves by whole texels when it does, and keeps its size while the camera turns.
  - Redraws: a cascade renders again only when its pose changes (snap, sun turn, caster depth), a caster change touches it, its last rendering missed a caster, or every view settles after the bake or the array changed. A still camera over still casters renders none.
  - Lookup: 1.5 texels of normal offset, a bias of 5 cm plus 2 texels, the first cascade whose rim holds the point, blended into the next over the outer 10% (or kept where the next misses), unshadowed past the last. Points past a cascade's far end count as at its far end, which is behind everything it holds.
  - Limits: one shadowed sun; fixed 1024² cascades (about 9 mm texels in the nearest one at 50 m) and a fixed σ 1.5 texel blur, so the penumbra doesn't widen with the sun's disk or the blocker's height as the baked sun's does; light bleeding where casters overlap along the rays, as with Henry's cubes; off for `RuntimeQuality::IntegratedGraphics`. The shadow removes the sun's direct light only: baked bounce and sky light stay under moving objects, as under the cubes' shadows.
  - Cost: one 4 MB depth and one 8 MB scratch texture and 24 MB of cascades at `Rg32Float`, made when a sun first has realtime shadows; up to three views per frame while the camera or the casters move; per fragment one or two moment samples per shadowed sun.
- The sky (`LightType::Sky = 11`, M20, `environment/`): light from an HDR image all around the scene. `SkyEnvironment { image, max_radiance }` on an entity requires a `DynamicLight::sky()` (intensity 1), so the sky is a light record like the others: its color tints, its intensity and effects scale it live, and turning the entity turns the sky. Henry has only a flat ambient color, which stays as it was. A sky light without an image is a uniform sky of its color: Henry's flat ambient as a light, with effects (and, from M21, baked visibility). It draws no background.
  - Image: equirect in Blender's mapping (the image's center looks along +X, +Y is up), 32-bit, 16-bit or 8-bit RGBA; Bevy's `exr` and `hdr` features load HDR files. When the image loads or changes, the CPU builds the sky off the main thread: a solid-angle-weighted mip pyramid, nine spherical-harmonic coefficients of irradiance (Ramamoorthi and Hanrahan 2001) projected from the level at most 512 texels wide, and a 128² specular cube with 6 levels for roughness 0 to 1, each texel the GGX lobe's average (Karis 2013's prefiltered half of the split sum) from 64 Hammersley samples read at their footprint in the pyramid (Křivánek and Colbert 2008). The build is deterministic: every texel is computed on its own, and the coefficients sum rows in order.
  - Shading (`surface/shaders/sky_light.wesl`): diffuse light is the coefficients evaluated at the normal, clamped at 0, times the sky's color and intensity. With Metallic shading the diffuse part is weighted by `(1 − F)(1 − metallic)` and the reflection is the cube at the mirror direction and level `roughness · 5`, weighted by Schlick's Fresnel with roughness (`F0 + (max(1 − r, F0) − F0)(1 − cos θ)⁵`); materials with their own reflection map keep it instead of the sky's reflection. The reflection is added after the material's occlusion texture, which darkens only diffuse and direct light, as Henry's environment reflection is.
  - `max_radiance` clamps the image for its light only. An HDRI's sun is far brighter than the rest of its sky (the test image peaks at 51 136 against a mean of about 0.23 with the sun clamped); left in, it would light every surface a second time, unshadowed, next to the sun light that stands for it. The background shows the full image.
  - Background: our own pass in `Core3d` after the opaque pass, before transparent ones: a full-screen triangle at depth 0 that draws where no opaque geometry did, from the equirect image's mips at the pixel's footprint. `NoSkyBackground` on a camera leaves its clear color.
  - Kept: lights keep Henry's model, and the sky is one more distant light: not in the light BVH or the cluster grid, visited by every lit fragment like a sun. In M20 it was realtime only (the bake snapshot skipped it); since M21 a sky that isn't `realtime` is baked (below). `ShaderFeatures::sky`. One sky lights the scene; with several, the first is used and a warning names the rest once.
  - Data: the coefficients live in the light buffer after the globals (§5), because the material bind group has no storage buffer to spare; the cube is bindings 30–31.
  - Limit: in M20 the sky had no shadows. It reached under roofs and into rooms as Henry's ambient does, and its reflection showed on occluded metal. M21 bakes its visibility (below); a realtime sky still has none. The 128² cube blurs mirror-like reflections a little (at roughness 0 a texel spans about 0.7°).
- Baked sky visibility (M21, `bake::sky`, `bake::sky_probes`): a sky light that isn't `realtime` is baked, in the last baked slots after the suns (`BakeId` order), and its shadow is how much of the sky a texel sees.
  - Bent: 8 bits per texel instead of Henry's 1, and the shadow of a hemisphere instead of a point. Each texel sends `BakeSettings::sky_samples` rays (64 by default, at most 16 in Preview bakes) over the hemisphere around its normal with a cosine density: the Sobol (0, 2) spread through the concentric disk, lifted onto the hemisphere (Malley's method), offset per texel by the stateless hash of the seed, mesh, light and texel. Rays start at Henry's 0.001 m offset, have no end and are alpha-tested when the light's transparency is on. The share that leaves the scene is the share of a uniform sky's light a Lambert surface there receives, rounded to the nearest of 256 levels (16 showed bands over the slow gradients of sky shadows). Triangles that see no sky at all drop the entry, like Henry's all-zero lights.
  - Kept: the share doesn't depend on the image, so turning the sky, swapping its image, and changing its color, intensity or effects all keep the bake (`light::placement` never drops a baked sky). Only a change to the static geometry needs a new bake, as for every light.
  - It is a shadow of the sky, not ambient occlusion: it scales the sky's diffuse light and its reflection alike (`open` in `add_sky`), and nothing else. Henry's ambient color, the material's occlusion texture and the other lights are untouched. Under an image the share multiplies the unshadowed irradiance at the normal, whichever part of the sky the occluders hide, so a texel that sees only the bright part of an uneven sky gets the mean of the whole.
  - Data: an 8-bit shadow word holds four texels. `shadow_row` reads four texels from a row at 8 bits, at most two loads. Only the High tier's Gaussian rows need eight texels: at 8 bits `row_binomials` reads the second four with a second row read, up to four loads per row where 1 to 4 bits take two. `baked_lighting::mirror` checks every tier against texel reads at 1, 2, 4 and 8 bits. The `.dynlight` format version 4 stores a sky as light kind 4, after the suns, and rejects lights out of that order and shadow bits other than 1, 2, 4 or 8.
  - Moving objects (`dynamic_geometry::sky_mirror`, `shaders/sky_probes.wesl`): the sky's counterpart of the sun maps is a grid of sky probes over the static bounds, `BakeSettings::sky_probe_spacing` apart (1 m by default, widened until the grid has at most 65 536 probes) and reaching half a spacing past them. Each probe traces `sky_samples` rays for each of the six axes, as a texel facing that axis would, and stores the six shares in 8 bits each: an ambient cube of sky visibility, 8 bytes a probe, in the aux buffer (§5). A probe whose rays reach the back of a triangle more than 25% of the time is inside geometry; it takes the mean of its valid face neighbours, one layer per pass, and probes no valid one reaches are open. A lookup blends the 8 probes around the point trilinearly (clamped to the grid) and weights the six values by the squared normal. Cost: 16 buffer reads per baked sky per fragment, only on meshes without bake data. Limits: the probes see static geometry only, so moving objects receive sky shadows but cast none and don't shadow themselves; and they blur sky shadows over about a spacing, so an object darkens under a roof but not in a gap narrower than the spacing.
  - Look: faces pointing down see the ground instead of the sky, so the undersides of tree crowns and the lower half of the chrome ball go dark. Nothing lights them from below, because neither the sky nor the sun bakes bounce light (see the suns' "Not baked" above). Henry's flat ambient color still lights them.
  - Cost: `outdoor`'s Full bake (30 meshes, 0.81 M texels) traces 67 M rays in 2.4 s and writes 917 KB, most of both the sky's: 64 rays per texel, 12 348 probes (4.7 M rays), and the 8-bit shadow rows. M19's 25 meshes took 0.6 s and 163 KB.
- Clustered realtime lights (`light_clusters`, M17). Henry's shader visits every realtime light at every pixel. We split the view of the camera the realtime lights are chosen for into 16 × 9 × 24 cells (depth slices exponential past 1 m, linear for orthographic cameras), and list in each cell the realtime lights whose sphere's view-space box covers it. A fragment visits only its cell's list.
  - Kept: every light is shaded by the same code, range check included; baked lights, the light BVH and baked shadows are untouched. A list holds its lights in buffer order, and the lights it leaves out add nothing, so a fragment sums the same terms in the same order as the flat loop.
  - The grid is a world-space partition: the shader projects a fragment with the clustering camera's stored matrix, not the current view's. Another camera reads the right lists inside that camera's frustum and visits every realtime light outside it. Fragments past the farthest light sphere visit none.
  - The flat loop stays for: no camera, fewer than `realtime_cluster_min_lights` realtime lights (8, §16), more than 2^20 list entries (logged once), or no light sphere in front of the camera.
  - Cost: one CPU build per frame, 0.14 ms for 1000 lights on one thread (`benches/clusters.rs`), and one more storage buffer, the 8th in the fragment stage (§5).
- Shadow cube faces in a render phase of our own (M18, §10) instead of a camera per face. Kept: Henry's shadow camera as six 90° faces at the light, the same casters, moments and blur, so the cubes hold the same images (a still `realtime_shadows` frame matches the face-camera path bit for bit). Only the CPU work per face changed.
- Fog: `VolumetricShape::Frustum` is a rectangular shaft along the light's forward axis, starting at the light's size and widening by `tan(outer_cutoff)` (clamped to 0–75°) over the fog radius. Box fog now turns with the light (Henry's and bevyout's ignored rotation). Both are a ray clipped against six planes in the light's frame.

## 13. Bevy 0.20 APIs

**Verified against 0.20.0 docs and examples:**

- **`Material` trait:**
  - methods `vertex_shader`, `fragment_shader`, `specialize`, `enable_prepass`, `enable_shadows`, `prepass_fragment_shader`
  - shaders are `.wesl`; the material bind group is `@group(constants::MATERIAL_BIND_GROUP)`
- **`ShaderBuffer`** (`bevy::render::storage`, formerly `ShaderStorageBuffer`): raw bytemuck bytes, `clear`/`extend`, resizes without rebuilding materials.
- **Material bind groups** are prepared once per material asset and don't follow the images they bind: replacing an `Image` under the same handle needs the materials marked modified (`surface::shared_arrays`).
- **`MeshTag` / `mesh_functions::get_tag(instance_index)`**, and `GpuComponentArrayBuffer` + its plugin.
- **Render passes** are systems in `Core3d` (`Core3dSystems::{Prepass, MainPass, PostProcess}`) using `ViewQuery`, `RenderContext`, `FullscreenShader`, and `ViewTarget::post_process_write`.
- **WESL module paths:** `bevy_pbr::render::{mesh_functions, forward_io, view_transformations, mesh_view_bindings}`, and `@if(...)`. `octahedral_*` moved to `bevy_render::utils`.
- **`GltfLoaderSettings::load_lights`.**
- **Crate licence:** `obvhs` 0.4.0 is MIT OR Apache-2.0.

**To verify in M1:**

- The wgpu primitive-index feature name, and `enable primitive_index;` through the WESL linker.
- `primitive_index` under Bevy's multi-draw indirect on each backend.
- The default vertex shader's instance-index output flag under WESL.
- `GpuComponentArrayBuffer` in a non-bindless material.
- The storage-buffer count per stage.
- `texture_cube<u32>` + `textureGather` and cube arrays through `AsBindGroup`.
- Whether the `Ready` event fires for glTF scenes.

## 14. Milestones

Sizes: S ≈ days, M ≈ 1–2 weeks, L ≈ 2–4 weeks.

- **M0 Setup + lift (S), done 2026-10-15:**
  - Rename the package to `dynlight` (lib + examples), create the VSA slice skeleton from AGENTS.md, add Bevy 0.20, `.cargo/config.toml` dev aliases, CI (fmt, clippy, test, `cargo-deny`), LICENSES, and `docs/PORT_MAP.md`.
  - Lift bevyout `core/*` into the `light/`, `effects/`, `fog/` and `shared/` slices; replace Unity RNG and Perlin; delete the Unity goldens and parity tests; add analytic and statistical tests.
  - `tests/repo_hygiene.rs`: 400-line file limit and forbidden identifiers (AGENTS.md).
  - Delivered beyond the list: a naga-checked WESL/Rust layout test for the GPU light (`shared::wesl_check`), a headless plugin test, and a gizmo-only `lighting_test` with `--frames`/`--screenshot` for smoke runs.
- **Status, 2026-10-15:** M0 to M9 are implemented (M10 and M11, beyond Henry's feature set, followed on 2026-10-16). Per milestone, below: what was delivered where it differs from the plan.
- **M1 Realtime-only lighting (M), done:**
  - `DynamicLight`, light buffer + globals, the `DynLitMaterial` Diffuse port, the WESL modules `types`, `math`, `light_types`, `light_processor`, `lighting` (brute-force + realtime loop).
  - All 8 light types, 15 effects, shimmer, cookies.
  - `lighting_test` example migrated; glTF conversion; gizmos.
  - Section 13 verification spikes.
- **M2 Baker core (L), done:** light BVH, MeshBuilder, occluder BVH, triangle builder, per-texel rays, optimize, seams/dilate, serialize, `.dynlight` container, stable IDs. Tests on synthetic scenes; benchmarks.
- **M3 Baked shadows at runtime (L), done:**
  - Asset loader, triangles/aux buffers, receiver records via `MeshTag`, `primitive_index` path, quality tiers, moved-light handling, stale detection.
  - Bake command + progress + debug views.
  - **This reaches parity with Henry's core feature.**
  - Delivered as `baked_lighting::ActiveBake` (applies a loaded `.dynlight`) and `bake::identity` (`ResolvedBakeId` on `BakeStatic` meshes and baked lights). Verified end to end in `lighting_test --bake`.
- **M4 Dynamic geometry (M), done:** light BVH traversal in the shader, distance cubes, Angular / DistanceCubes modes.
- **M5 Bounce + transparency (M), done:** CPU photon cubes, bounce bake / compress / decode, alpha-tested shadow rays. Alpha masks come from `DynLitMaterial` (`surface::material_alpha`: mask cutoff, or 0.5 for blended modes).
- **M6 Metallic + Unlit + normal maps (M), done:** BRDF from formulas, optional own reflection cubemap.
- **M7 Volumetric fog (S), done:** migrate the bevyout pass. A `Core3d` post-process system after tonemapping, enabled per camera with `FogPostProcess`.
- **M8 Realtime VSM shadows (L), done:** spike A was enough: six depth-prepass cameras per shadowed light, moments + blur passes in `Core3d` between the prepass and the main pass, one cube array bound to every `DynLitMaterial`. Budget-limited, nearest lights first; off for `RuntimeQuality::IntegratedGraphics`. Example: `realtime_shadows`.
- **M9 Fallback + polish (M), done:** triangle-ID attribute mode for adapters without primitive index, light presets, Feathers inspector example, docs, a 1000-light stress test.
  - Fallback: see §6. Presets: `DynamicLight::{point, spot, discoball}` and `light::sun_light`, from Henry's editor menu.
  - `inspector` example behind the `inspector` Cargo feature (Feathers needs Bevy UI, BSN scenes and custom cursors).
  - `stress_lights`: 1000 baked lights 1.43 ms per frame; 100 realtime 1.5 ms; 1000 realtime 6.0 ms (RTX 4080 SUPER, dev profile, 1280×720). The bake of that scene takes 0.7 s. (Superseded by the 2026-10-15 demo rework below.)
  - `gpu_check`: GPU readback against `light::mirror` at 1024 pixels; the largest difference is 0.001.
  - Debug views (the M3 item): `DynamicLightingSettings::debug_view` = `Lighting` (white surfaces) or `LightCount` (heat map of the lights reaching each fragment), passed as light-buffer flags. `lighting_test --debug-view …` or the V key.

- **WebGPU publishing, 2026-10-15:**
  - Replaced the fork's Unity package in a normal fast-forward commit, keeping Henry's history and MIT notice; the default branch stays `master`.
  - Bakes run in the browser: `BakeRequest::output` is optional and `BakeReport::handle` is the in-memory `DynLightBake`; bake timing uses `bevy::platform::time::Instant`. The occluder BVH is built from `obvhs`'s PLOC, reinsertion and CWBVH steps directly, because its `build_cwbvh_from_tris` helper times itself with `std::time::Instant`, which panics on `wasm32-unknown-unknown`. The steps and the output are the same as before. Rayon runs its parallel iterators on the calling thread there.
  - Lit materials are marked modified whenever the shadow cubemap or cookie array is replaced (`surface::shared_arrays`): Bevy prepares a material's bind group once and doesn't track its images. In the browser the materials were prepared before the first shadow cubemap and kept sampling the zeroed 1×1 placeholder, so every realtime-shadowed light was black.
  - Examples share `examples/support/` for the page canvas, status line and arguments; in a browser, the query string stands for the command line (`?debug-view=light-count`). `lighting_test` and `inspector` bake a Preview at startup in the browser (lower texel density, no bounce); `stress_lights` defaults to 256 realtime lights there and writes its frame time into the page.
  - `web/build.sh` builds each demo with the `wasm-release` profile, runs `wasm-bindgen` (version checked against `Cargo.lock`) and Binaryen's `wasm-opt -Oz`, and writes `dist/` with the landing page and native-screenshot thumbnails from `web/thumbnails/`. `.github/workflows/pages.yml` deploys it on pushes to `master`.
  - Demo rework (after user feedback). The lighting test's camera orbited behind its wall for half of every lap; now `examples/support/orbit.rs` gives the demos a drag-to-orbit, scroll-to-zoom camera that drifts when idle, kept in front of the wall there. In the lighting test you drive a cube with WASD or the arrow keys (Bevy's `KeyCode` is the key position, so Colemak's WARS works). It carries a realtime-shadowed lantern that Space switches. The fog volumes moved to the sides, out of the camera's line of sight. `stress_lights` now has a low orbiting camera, jittered lights with 0–2 pillars 0.8–1.3 m from each, and a 4 m radius; in realtime mode the lights drift. New numbers (same machine and profile): 1000 baked lights 1.79 ms, 100 realtime 1.88 ms, 1000 realtime 11.1 ms; the 1000-light bake (1047 meshes, 6.3 M texels, 18 M rays) takes 2.0 s.
  - Realtime shadow cubemaps are too expensive for the stress scene: each shadowed light costs about 4 ms of CPU per frame in the dev profile, about 0.7 ms for each of its six depth-prepass face cameras. The GPU pass timings stay small and the cost grows with the scene's entity count, so it is per-view CPU work; it hasn't been profiled further. With 256 lights, 16 shadowed lights took the frame from 2.5 ms to 73 ms, so the stress scene stays unshadowed in realtime mode. Fixed in M10: cubemaps now redraw only the faces that changed.
  - Checked in headless Edge 155 with WebGPU on an RTX 4080 SUPER, with the published build: all four demos run. `lighting_test` bakes its Preview in 91 ms (103,188 texels, 19,277 rays) and the inspector in 4–8 ms. B, V, the inspector's sliders, buttons and checkboxes, and the query-string options work. After `wasm-opt`, each demo is 21.8 MB (6.9 MB gzipped); the inspector, with Bevy UI, is 31.9 MB (9.9 MB).

- **M10 Shadow cubemap cache, done 2026-10-16:**
  - Stable cube slots, per-face dirty tracking and moving-only casters for lights at their baked position (§12, Beyond Henry). `ShadowCubemapStats { lights, faces_rendered }` reports the cost.
  - `realtime_shadows` (two shadowed lights, RTX 4080 SUPER, dev profile, 1280×720): while the crates and lamp move, 9 of 12 faces render per frame, 7.7 ms; with `--still`, 0 faces, 2.35 ms. Before the cache all 12 faces rendered every frame.
  - Follow-up after M14: casters whose transform is set to the same value count as still. The lab's cube is set every frame by its driving system, so the panel redrew one face per frame while the cube stood still; now none (idle lab at 2560×1440, vsync off: 2.32 ms per frame against 3.30 ms, median of five alternating runs).
- **M11 Area lights and a square fog shaft, done 2026-10-16:**
  - `LightType::Rectangle = 8` and `Tube = 9`, `DynamicLight::size`, presets `rectangle(w, h)` and `tube(length)`; half extents in `gp1`/`gp2`, orientation from `up`/`forward`, reach past the shape in `radius_sqr`. Shading, gizmos, inspector sliders and glTF `dynlight_type`/`dynlight_size` extras.
  - They follow Henry's rules (§12, Beyond Henry): 1-bit shadows from the light's position, `radius` around the position, only moving a baked light invalidates it. The bake, the `.dynlight` format (still version 1) and the shadow decoding are unchanged. M12 then bakes them from several points.
  - `VolumetricShape::Frustum` and rotating box fog; glTF `fog: "frustum"`.
  - Tests: the area math against brute-force integration, nothing lit past the radius around the position, area lights baking the same bytes as a point light, baked area lights keeping their bake when turned or resized, and the shaft fog shape. `gpu_check` adds a tilted panel and a tube (largest difference 0.001).
  - The lab gets a fluorescent ceiling panel (`FluorescentStarter`, a frustum fog shaft, realtime shadows for the cube you drive) over a crate and a pillar, and a flickering wall tube. Its bake (12 meshes, 29 lights, 7.9 M texels, 2.1 M rays) takes 0.48 s.

- **M12 Soft area-light shadows within Henry's rules, done 2026-10-16 (replaced by M14):**
  - Area lights bake as `shadow_points` point lights along their long side (§12, Beyond Henry), packed into one slot each with an equal share of the light; `BakeId::part` gives each point its id. The `.dynlight` format stays at version 1.
  - Shadow cubemap faces take their blur width from the face uniform; area lights blur wider (σ up to 4 texels). Point lights keep the 5-tap σ 1.5 kernel.
  - glTF `dynlight_shadow_points`, an inspector slider, and gizmo crosses at the shadow points.
  - Tests: the point layout and ids, packing (each record finds the shape's center, and the shares add up to the whole light), placement (every point must be in place), the bake (one point equals a point light; three points bake three lights with a penumbra), and the blur kernel against the mirror.
  - The lab panel uses 5 points. Its bake (12 meshes, 39 lights, 7.9 M texels, 4.2 M rays) takes 0.47 s.

- **M13 Blender-like area-light shadows, done 2026-10-16 (replaced by M14; the row reads stay):**
  - Panels spread their shadow points over both axes in a lattice with one column and one row per point; tubes keep their row. Up to 16 points (glTF extras, inspector slider). Records find the shape's center from a 2D offset (`gp3` and the `cookie_index` bits).
  - The bake keeps an area light's points in consecutive slots (light BVH items, `BakeLight::point_of`); records carry the group in channel bits 20–31, and the lighting loops shade each area light once and add up its points' shadows.
  - Shadow bits are read a row at a time: the same results with 16 loads per light at High (was 400) and 8 at Medium (was 16); the High filter no longer reads outside a triangle's data above its grid (§12, Fix).
  - Tests: the lattice (own column and row, centered, stable, no diagonal), 2D-offset records and `gpu_check` with an offset panel record, the row reads against texel reads at every tier, the grouped light BVH (consecutive slots, queries against brute force), the group bits and `point_group`, and the bake (consecutive slots; a square panel's shadows shift along both floor axes).
  - The lab panel uses 16 points. Its bake (12 meshes, 50 lights, 7.9 M texels, 7.5 M rays) takes 0.50–0.59 s, within the spread of the 5-point bake (39 lights, 4.2 M rays, 0.49–0.59 s) on the same machine and day. Looking down at the panel's props at 2560×1440 (RTX 4080 SUPER, Vulkan, dev profile, vsync off; median of five runs alternating with the previous commit), a frame takes 5.0 ms at High and 4.0 ms at Medium, against 6.1 ms and 5.2 ms with 5 points before. The material pipelines compile as fast as before (about 1.8 s after startup in the lab): each lighting loop shades once per pass, with one extra pass for the last gathered light, so the drivers inline the shading no more often than before. A Cycles render of the same panel, pillar and crate shows the same contact-hardening shape; ours keeps faint terraces far from the pillar.

- **M14 Soft area-light shadows, done 2026-10-16:**
  - Replaces M12 and M13's shadow points: `light::shadow_points`, `DynamicLight::shadow_points`, glTF `dynlight_shadow_points`, the inspector slider, gizmo crosses, `BakeId::part`, grouped slots and the group bits are gone. An area light is one record at its center (§12, Beyond Henry).
  - Bake: 4-bit visibility per texel over the shape (`bake::area`, `bake::shadow_grid`), `BakeSettings::area_light_samples`; `.dynlight` format version 2. Runtime: N-bit row reads and channel bits 20–22. Moving objects: the soft distance-cube lookup for area lights (`dynamic_geometry::mirror`), with the cube bias moved into the shader.
  - Tests: the bake (a panel within 0.02 mean of a brute-force trace with at least 12 levels; tubes soften only along their length, turned or not; point lights keep 1 bit), N-bit rows against texel reads, the soft lookup (against the traced share within 0.075, no steps, wider penumbras for larger lights and farther blockers, empty space past the radius), and `gpu_check_area`, a baked and an unbaked floor under a panel and a tube read back and compared with the CPU mirrors at every tier (largest difference 0.0001; ignored test in `gpu_examples`).
  - The lab panel bakes with 128 samples: 12 meshes, 29 lights, 7.9 M texels, 6.9 M rays, 0.83 s, 340 KB (with 16 shadow points: 50 lights, 7.5 M rays, 0.57 s, 497 KB). Looking down at the panel's props at 2560×1440 (RTX 4080 SUPER, Vulkan, dev profile, vsync off; median of five runs alternating with M13's commit), a frame takes 2.57 ms at High and at Medium, against 4.28 ms and 2.68 ms with 16 shadow points; the default view is unchanged (about 3.2 ms). The soft lookup adds shader code, so the material pipelines finish compiling about 1.9 s after startup in the lab (1.7–1.85 s before); its filter calls the bilinear lookup from one place to keep the shader small.
- **M15 Rectangle-light specular by linearly transformed cosines, done 2026-10-16:**
  - Prompted by Bevy 0.20's `RectLight`, which shades with the same method (and has no shadows). Nothing of Bevy's lighting is used: the table comes from the authors' repository, the shader is written from the paper, and `light::AREA_LIGHT_LUT` is our own image (§12, Beyond Henry).
  - `light::ltc` (table reads like a linear sampler, the quad integral, `rectangle_specular`), `light/shaders/ltc.wesl`, `area_ltc_specular` in `area.wesl`, the table on `DynLitMaterial` (`area_light_lut`, bindings 27–28), and `surface::mirror`, a CPU reference of Metallic shading's per-light specular term.
  - Tests: the table's layout and range, lookups at texel centers, the identity transform against Lambert's form factor (within 4 · 10⁻⁶), the quad integral against the transformed lobe integrated over the panel (within 1%, clipped and head-on views included), the fit against brute-force GGX, front-only reflection, the fallbacks, small panels against point lights, and wider highlights for larger panels. `gpu_check_specular` reads back a copper floor at two roughnesses under two panels, a tube and a point light from a low camera (view angles up to about 70°) and compares 4032 samples with the CPU mirrors (largest difference 0.0032; ignored test in `gpu_examples`); with the shader's LTC path turned off it fails 1191 samples.
  - The shader code is larger: the material pipelines finish compiling about 0.1 s later in the lab (median 2.34 s against 2.22 s after the plugins finish, three alternating runs with the LTC path removed). `gpu_check_specular` therefore retries its readback until it matches or 20 s pass, like `gpu_check_area`; one run checked once after 2 s had read an unlit frame. Frame time wasn't measured: the lab's only Metallic surfaces are the sphere row, which no rectangle reaches.
- **Comparison with Bevy's lights, 2026-10-16.** A harness outside the repo (it spawns Bevy's `PointLight`, which the repo bans) renders the `stress_lights` layout with either system (release profile, 1920×1080, RTX 4080 SUPER, 512² shadow cubes for both, medians of alternating runs).
  - Baked lights cost the same GPU time as Bevy's lights without shadows (1000 lights: 1.09 ms against 1.07 ms), because both shade every nearby light per pixel; baking makes the shadows nearly free. Bevy with 100 shadowed lights takes 77–122 ms per frame.
  - Two losses: realtime lights are looped without culling (1000 realtime lights: 22.6 ms of GPU against 1.08 ms), and moving realtime shadow cubemaps were CPU-bound on the per-face cameras (16 shadowed lights moving: 53–76 ms against Bevy's 18–28 ms). M16–M18 address them.
- **M16 Face cameras without light clustering, done 2026-10-17:** `realtime_shadows` removes Bevy's `Clusters` from its face cameras, so Bevy no longer clusters lights on the GPU, prepares lights or builds a mesh view bind group for them. Harness, 16 moving shadowed lights (96 faces per frame): 26.7 ms per frame against 53–76 ms (Bevy 16.1 ms in the same run); still: 1.72 ms.
- **M17 Clustered realtime lights, done 2026-10-17:** new slice `light_clusters` (§9, §12 Beyond Henry), binding 29. Harness, moving realtime lights, GPU time of the main pass with the grid off → on (Bevy):

  | Realtime lights | Grid off | Grid on | Bevy |
  |---|---|---|---|
  | 8 | 0.203 ms | 0.194 ms | |
  | 16 | 0.238 ms | 0.201 ms | 0.099 ms |
  | 32 | 0.320 ms | 0.202 ms | 0.117 ms |
  | 100 | 0.817 ms | 0.238 ms | 0.161 ms |
  | 1000 | 23.2 ms | 1.71 ms | 1.09 ms |

  Frame time at 1000 lights: 23.7 → 2.16 ms (Bevy 1.65 ms). Not measured yet: how much of the remaining gap is per-light shading cost and how much the looser assignment (a box per light, where Bevy tests light spheres against cluster planes). Checks: a property test (every light whose sphere holds a random point is listed in its cell, perspective and orthographic), and GPU readback of 200 lights through `gpu_check --many` against `light::mirror` (largest difference 0.0019; shrinking the spheres by half in the assignment fails 65 of 1024 samples).
- **M18 Shadow faces in our own render phase, done 2026-10-17:** §10 Spike B replaces the face cameras (`views`, `casters`, `extract`, `phase`, `queue`; `cameras.rs` and `filter.rs` are gone). Public API, `vsm.wesl` and the cube array are unchanged.
  - Harness, 16 moving shadowed lights (96 faces per frame): median 16.8 ms per frame over five alternating runs (17.1 ms in an earlier set of three), against 26.7 ms after M16 and 53–76 ms before; Bevy 16.3 ms in the same runs (22.0 ms in the earlier set). Still: 1.66 ms. What's left per face is one `Core3d` schedule run, which Bevy pays for each point-light face too.
  - Still cubes settle 0.25–0.32 s after start (the last frame with a face rendered in `realtime_shadows --still`), against 1.95 s with the face cameras, whose global "pipelines compiling" check redrew every cube; the example's warm-up went from 2 s to 1 s.
  - Checks: a still `realtime_shadows` frame matches the face-camera build bit for bit (1280 × 720, largest difference 0); unit tests for the caster lists and the per-face redraws; app tests that a caster no camera sees is marked visible and listed only in the face looking at it, and that a face that stops rendering loses its view; WebGPU in Edge (9 of 12 faces per frame, as on native).
  - README benchmarks (10 alternating runs after M18, runs capped at 60 fps by the window system left out of the sub-16 ms rows): 1000 baked lights' main pass took 0.86 ms against 1.09 ms the day before, with a bit-identical image of the 100-light scene; the cause wasn't pinned down. Moving shadows came out at 18.3 ms against Bevy's 19.4 ms in this set.
  - Found on the way: Bevy keys meshes in the render world by main entity (their render entity is `Entity::PLACEHOLDER`), and `DrawMesh` silently skips draws until Bevy's preprocessing pipelines have compiled, which is why the draws report their own skips.
- **Comparison with Henry's package in Unity, 2026-10-17.** A Unity 6000.6.4f1 project outside the repo (Built-in pipeline, IL2CPP release player, Henry's package unchanged at e2a51ad) builds the harness's scenes from an exported scene description: Z mirrored, winding flipped. It bakes them with his `Raytrace(2048)`. All three renderers run on Direct3D 12, 1920×1080, 4× MSAA; medians of 10 alternating runs, numbers in the README.
  - Henry's realtime shadows switch off for good after a frame that fills all 16 cubemaps (§12, Fix), so the shadow scenes use 15 shadowed lights.
  - Images: rendered without tonemapping and with time frozen, the stress, light-type and shadow scenes differ from Henry's on average by 0.01–0.14 of an 8-bit step; at most 0.2% of pixels differ by more than 16 steps, all on pillar edges. Henry's realtime and shadow scenes are left unbaked, because the harness's realtime modes shade every mesh on the dynamic-geometry path.
  - Frame time: Unity's player needs about 0.4 ms per frame, Bevy about 2 ms in every scene. On Direct3D 12 Bevy's frame is about 0.6 ms longer than on Vulkan.
  - Henry wins baked lights on the GPU: his whole frame takes 0.46 ms against 0.93 ms for our lit pass alone (0.84 ms after the fixes below). Not explained yet. Two candidates from reading the code: overdraw, since Bevy's binned `Opaque3d` phase doesn't sort front to back and Unity's Built-in pipeline does, with no depth prepass in either; and one large shader with every lighting path behind runtime branches, where Henry compiles a variant per quality and feature combination.
  - Experiments, 2026-10-17 (harness `exp_baked.sh`, 1000 baked lights, Direct3D 12 with DXC, Medium quality, lit pass medians):
    - They first turned up a regression: the FXC fix (§15) had made the lit pass 3.7× slower on DXC, 3.49 ms against 0.93 ms. DXC keeps a module-scope `var<private>` array as a zero-initialized global in per-thread scratch memory, for every pixel, baked or not. The light BVH stack is now a function variable of the caller (`BvhStack`, passed by pointer), which both compilers keep local: 0.93 ms again.
    - Shader size isn't it: with the unbaked-mesh and area-light paths stripped, the pass takes 0.91 ms against 0.93 ms.
    - Early depth testing was off: the lit and prepass fragment shaders always contained the alpha-mask `discard`. Behind Bevy's `MAY_DISCARD` define (set for `AlphaMode::Mask` only) the pass takes 0.80 ms. A depth prepass on top saves nothing: 0.74 ms plus 0.08 ms for the prepass. Bevy's pass gains 0.39 ms from one (1.35 to 0.96 ms), which leaves its frame slower than without.
    - Rejected: rewriting the Medium and High shadow filters (`shadow_bilinear`) with `vec4` rows instead of local arrays. High got 0.12 ms faster (1.43 to 1.31 ms), but Medium, the default, 0.03 ms slower, because the unrolled High path raises register pressure for the whole shader. A running-sum loop was slower at both (Medium 1.05, High 1.78 ms). Worth trying again once quality is a shader define, like Henry's keywords.
    - Found on the way: Bevy declares the prepass `FragmentOutput` only for prepasses with color targets, so an alpha-masked `DynLitMaterial` failed to compile in depth-only prepasses, the realtime shadow pass among them. `dynlit_prepass.wesl` now has an output-less entry point for those, as Bevy's own prepass shader does.
    - README remeasured (10 alternating runs): lit pass 0.84 ms against Bevy's 1.31 ms; frames 1.99 ms for both.
    - Shader variants (§7.3), harness `exp_variant.sh`, 10 alternating runs against the build without them, lit pass medians: Medium 0.81 → 0.69 ms, High 1.45 → 1.35 ms, Metallic 0.91 → 0.85 ms. With only the paths this scene uses (`--lean`: every `shader_features` entry off) 0.54 ms, with a wider spread (0.48–0.64 ms), and an identical image. Frames stay at about 2.1 ms. That leaves Henry's 0.46 ms whole frame close; the `vec4` shadow-filter rewrite above can now be tried for High alone.
    - Found on the way: lights without an explicit `BakeId` seeded their effect randomness with their entity id, so adding three resources shifted every flicker and failed the baselines. They are now seeded from their `Name` path and position (AGENTS.md, "No entity ids in output").
  - Performance round 2, 2026-10-17, after an outside review. Nine changes, one commit each, measured as cumulative checkpoints with the harness's `exp_ab.sh` (two builds from worktrees, alternating) and `exp_chain.sh` (each checkpoint against the one before, 5 runs). All of them together against the build before, 10 runs, lit pass: baked 0.90 → 0.62 ms, High 1.84 → 1.26 ms, 1000 realtime lights 2.07 → 1.46 ms; frames 2.5 → 1.9 ms with realtime lights. Images unchanged (baselines).
    - Bevy's own light clustering moves to the CPU with a single cell (`light_clusters::bevy_clusters`). Cameras keep Bevy's `Clusters`, because every material's view bind group binds its buffers, but Bevy no longer runs its clustering compute pass each frame (0.05 ms of GPU time). The lit pass also measured 0.2–0.3 ms faster, but that was draw order (below).
    - A realtime light is listed only in the cells its sphere reaches, not in the whole block of cells around its box: half the entries, 1.79 → 1.47 ms for 1000 lights. The narrow phase first tripled the CPU build (162 → 467 µs); per-axis cell bounds and row runs brought it to 270 µs.
    - Shadow filters: Medium counts one-bit texels as bits, and the High and multi-bit rows are `vec4`s (the rewrite rejected above). Per shaded fragment the bit count makes Medium about 8% faster; the `vec4` rows change nothing measurable. `ShaderFeatures::bounce` changes nothing either while bounce is compiled in: its build compiles to byte-identical DXIL.
    - Corrected 2026-10-18: most lit-pass steps of this round were draw order, not shader code. Bevy's opaque phase has no depth sort, and its draw order changes from build to build (stable within a build), so the lit pass shades about 2.95 M or 3.83 M fragments per frame in the baked scene (`main_opaque_pass_3d` fragment-shader invocations, Vulkan pipeline statistics). That alone is ±0.2 ms. The bit count, the `vec4` rows, `bounce` and the clustering step were first credited to register allocation; the counts show they landed on the other draw order. Per fragment: 0.236 ns before the round, 0.21 after; the cell sphere test is real (same fragment count, 1.79 → 1.47 ms). Lit-pass A/B runs now use `--prepass`, which shades a constant 2.64 M fragments, and print `bench invocations` to check both builds draw the same.
    - No measurable change in the harness scenes, kept because they remove per-frame work: shadow-face bind groups made once per frame, faces without casters cleared instead of drawn, and per-frame buffers that keep their GPU size (a `ShaderBuffer` that changes size gets a new GPU buffer, and Bevy rebuilds the bind groups of every material binding it).
    - README remeasured (10 alternating runs): baked lit pass 0.61 ms against Bevy's 1.33 ms, and 0.45 ms with `--lean` against Henry's 0.46 ms whole frame; 1000 realtime lights 1.45 ms against Bevy's 1.33 ms.
  - 1000 realtime lights: Henry's shader visits every realtime light at every pixel; his whole frame takes 6.1 ms of GPU time against 1.45 ms for our lit pass.
  - Moving shadows (90 faces per frame): Unity redraws them in about 7 ms per frame, dynlight takes 19.8 ms and Bevy 22.9 ms; the cost is Bevy's per-view schedule run (M18). Henry redraws every face every frame, so his still scene costs the same 7 ms; ours costs 1.9 ms.
  - Bake of the 1000-light scene, direct light only for both: 1.3 s (byte-identical over three runs) against 22.2–22.6 s for Henry's baker in the editor.
- **M19 Baked sun, done 2026-10-18:** the first of four milestones for outdoor light (M20 an HDRI sky, M21 baked sky visibility, M22 realtime sun shadows). §12, Beyond Henry, has the design.
  - Light: `LightType::Sun`, `DynamicLight::sun()` and `angular_diameter`; `sun_light` returns a sun. The light-buffer header grows to 48 bytes with the distant-light counts (§5). `ShaderFeatures::sun`; glTF `dynlight_type: "sun"` and `dynlight_angular_diameter`; an inspector slider; a gizmo ring with arrows.
  - Bake: `BakeLight::shape` (`light::LightShape`: point, area or sun) replaces the area-light option; `bake::texel_shadow` dispatches on it, `bake::sun` traces the disk, and `bake::area::adaptive_share` is the probe-then-spread loop both soft shapes share (area-light bakes are byte-identical). `bake::sun_map` bakes the sun maps; `.dynlight` format version 3; the aux buffer gains the sun maps (§5).
  - Tests: hard sun shadows against the parallel projection of a box, penumbras that widen with the disk and the blocker's height, suns kept out of the light BVH, bounce and distance cubes, determinism with two suns, the format's light order and sun-map count, sun maps holding the first surface along slanted rays, the lookup mirror (roofs, edges ramping over three texels, resting receivers lit), the shader against the mirror (naga), sun placement (moving keeps the bake, turning or a new disk drops it), packing and the cluster grid. `gpu_check_sun` reads back a baked and an unbaked floor under a 3° sun and four boxes and compares 3862 samples with `baked_lighting::mirror` and `dynamic_geometry::sun_mirror` at the integrated and high tiers (largest difference 0.0010; a nearest-texel lookup in the shader fails 22 samples).
  - Example `outdoor`: a 40 m courtyard with a house, a shed, a wall, a pergola, three trees and a lamp post with a baked lamp under a 1° sun, and a ball and a crate that aren't baked, standing in the pergola's and a tree's shadows. Its Full bake (25 meshes, 0.79 M texels, 15.4 M rays) takes 0.53–0.63 s and 163 KB; Preview 0.13 s. New baselines `outdoor`, `outdoor_unbaked`, and `outdoor_fxc` on Windows against `outdoor`; the existing baselines are unchanged. `lighting_test`'s row of light types leaves out the sun, which lit the whole lab and shifted every hue.
  - Found on the way: obvhs's ray-box test missed rays with a −0 direction component, such as `(−0, −1, −0)`. It takes a negative inverse for −0 but picks the near and far planes by `direction < 0`. `bake::occluder::segment` adds +0 to the direction; a regression test covers it.
- **M20 HDRI sky, done 2026-10-18:** new slice `environment` (§3); §12, Beyond Henry, has the design.
  - Light: `LightType::Sky`, `DynamicLight::sky()`, `SkyEnvironment`, `NoSkyBackground`, `SkyStatus` (a build is running). The light buffer gains the 144-byte sky block after the globals (§5), `DynLitMaterial` the specular cube at bindings 30–31. `ShaderFeatures::sky`. glTF extras and the inspector leave the sky out, since it needs an image; gizmos skip it.
  - Build: 25 ms for the 1024 × 512 procedural sky and 51 ms for a 2048 × 1024 HDRI (dev profile, 32-thread desktop CPU), off the main thread. `--capture` waits for it.
  - Example `outdoor`: a procedural sky matched to the sun (blue overhead, paler at the horizon, a glow and a disk around the sun, a dim ground), or `--hdri path` for an `.exr` or `.hdr` file, scaled to the procedural sky's mean radiance with its sun clamped and turned so its sun lies in the sun light's direction. A chrome ball (roughness 0.05) and a brass one (0.35) show the reflection. The examples take Bevy's `exr` and `hdr` features as a native dev-dependency; the crate doesn't need them.
  - Tests: the mapping (Blender's, round trips, wrapping at the seam), the pyramid keeping the mean radiance, the coefficients against the irradiance integral of a smooth sky, a uniform sky staying uniform through the prefilter, rough levels spreading a bright spot, the sampled lobe against GGX, image formats, the cube's faces and levels, the sky frame turning with its entity, the background finding the realtime sky, the build and a rebuild when the image changes in a headless app, the clamp limiting the light but not the background, the light-buffer layout, and the shaders against the CPU through naga. `gpu_check_sky` reads back a Diffuse and a Metallic ball (roughness 0.3) under a turned and tinted sky with the background behind them, and compares 966 diffuse, 736 reflection and 2136 background samples with `environment::mirror` (largest difference 0.06 of the tolerance; with the sky frame's sign flipped in the shader it fails 1521 samples).
  - Baselines `outdoor` and `outdoor_unbaked` re-blessed for the intended change: the sky lights the shadows blue-gray where they were nearly black, the sky background replaces the clear color, and the two metal balls are new. The other baselines are unchanged; `outdoor_fxc` matches the new `outdoor`.
- **M21 Baked sky visibility, done 2026-10-18:** §12, Beyond Henry, has the design.
  - Light: `LightShape::Sky`; a sky is baked unless it is `realtime`, and turning it or swapping its image keeps the bake. The background finds a baked sky as well as a realtime one.
  - Bake: `bake::sky` traces the texels' cosine-weighted rays and `bake::sky_probes` the probe grid; `BakeSettings::sky_samples` (64, Preview 16) and `sky_probe_spacing` (1 m). Distant lights are ordered suns, then skies. `.dynlight` format version 4 (`bake/format/lights.rs` and `maps.rs` split off `format/mod.rs`); the aux buffer gains the sky probes (§5).
  - Runtime: 8-bit shadow rows in `triangles.wesl`, `dynamic_geometry/shaders/sky_probes.wesl` for meshes without bake data, and `add_sky` scaled by the openness on both paths. Realtime skies stay unshadowed.
  - Tests: cosine-weighted directions (mean cosine 2/3), a raised horizon leaving cos² of its height, a roofed floor seeing almost no sky (and an enclosed one dropping its entry) against an open floor seeing all of it, a long wall hiding the analytic share `½(1 + d/√(d² + h²))` per texel and column, the probe grid's bounds and cap, probes under a roof against the roof's view factor, probes inside geometry filled from above, no probes in `LightingOnly` or empty scenes, determinism with a sky and probes, the format's light order and probe validation, 8-bit rows at every filter tier in `baked_lighting::mirror`, the probe lookup's mirror (`dynamic_geometry::sky_mirror`), and both shaders compiling through naga with the format's constants. `gpu_check_sun` now adds a uniform sky and checks its baked and probe shadows next to the sun's at all four quality tiers (largest difference 0.0010; without the sky shadow in the mirror 921 samples fail).
  - Example `outdoor`: a porch roof on two posts in front of the house. Its Full bake takes 2.4 s and 917 KB (§12).
  - Baselines `outdoor` and `outdoor_unbaked` re-blessed for the intended change: the baked sky darkens the porch, the bases of walls, the undersides of the tree crowns, the shed's sun shadow and the chrome ball's lower half; on the unbaked view the probes shade the crate under the tree; the porch is new. Every `lighting_test`, `realtime_shadows` and `stress_lights` baseline is unchanged (1-, 2- and 4-bit shadows read as before); `outdoor_fxc` matches the new `outdoor`.
- **M22 Realtime sun shadows, done 2026-10-18:** the last of the four outdoor milestones; §12, Beyond Henry, has the design.
  - Runtime: `realtime_shadows::cascades` (fit, snap, redraw plan), `sun` (the shadowed sun, cascade views and their casters), `targets` (the render targets, split off `render.rs`), `sun_vsm.wesl` and its mirror `sun_mirror`. `ShadowMode::Realtime` on a sun now means cascades; `DynamicLightingSettings::sun_shadow_distance`; `ShadowCubemapStats::cascades_rendered`. The light buffer gains the 208-byte sun-shadow block (§5), `DynLitMaterial` the cascades at binding 32. The cube-face pass serves both through `ShadowTarget`.
  - Found on the way: a caster that became baked kept its unbaked bounds in the caster tracking, so cubes and cascades of lights in place counted it as moving until it moved. It is now reported when `BakedReceiver` is added or removed.
  - Tests: the split depths; each cascade holding its slice inside its rim; the GPU rows against the cascade's view and projection; small camera moves keeping sizes and moving centers by at most one step; the depth reaching casters over the cascade; the redraw plan (first rendering, still, a change, a baked change in moving-only cascades, a miss, a reset, a snap); what a cascade draws; the mirror shadowing the ground under a box, past the far end and outside the cascades; the terminator rule for static receivers; cascade extraction as orthographic views; baked casters' bounds; the light-buffer layout; and the shader compiling through naga with the mirror's constants.
  - Example `outdoor`: the sun has `ShadowMode::Realtime`, so the ball and the crate cast realtime shadows onto the baked courtyard, and without a bake everything casts them; `--camera` holds the camera still (shared with `lighting_test` in `examples/support`).
  - Baselines: new `outdoor_moving_shadow`, the ball's shadow next to the pergola's baked stripes. `outdoor` re-blessed for the ball's and the crate's new shadows (374 pixels), `outdoor_unbaked` for the sun's shadows of every caster. Every other baseline is unchanged; `outdoor_fxc` matches the new `outdoor`.
- **M23–M26 Ideas from Wicked Engine, started 2026-10-18:** from Turánszki's BGF2026 talk and Wicked Engine's source (MIT). No code is ported, so `PORT_MAP.md` and `LICENSES/` are unchanged. Two outside reviews checked the plan against Bevy 0.20. Each step stays only if a measured A/B shows it faster.
  - The plan:
    - M23: cube faces that no camera can see wait until one can.
    - M24: one render-world driver preprocesses and draws every shadow view. Today each face runs Bevy's whole `Core3d` schedule, with four command encoders and two empty compute passes.
    - M25: a subgroup fast path for the light loops.
    - M26: a depth prepass by default, only on a clear win.
  - Outcome: M24 kept (moving shadows 22.6 → 8.6 ms per frame); M23, M25 and M26 measured and dropped.
  - Fixed first: shadow cubemaps, realtime lights and fog volumes were picked with the cameras' far plane, which Bevy's infinite reverse-Z projections never clip at. A light past a camera's `far` (1000 m by default) was dropped even though it reaches what the camera shows. `shared::sees_sphere` ignores the far plane, like Bevy's own culling, and `repo_hygiene` keeps `intersects_sphere` out of every other file. Images are unchanged, since no example sets `far`.
  - M23 dropped: in the moving-shadow benchmark (15 lights, 90 faces per frame), none of the faces that render is out of every camera's view. Every shadowed light's center lies inside the view, and from there all six faces can be read. The face test was measured, not shipped: per face, a five-point hull (the light and the corners at its radius, two texels wider for the seam) against the cameras' side and near planes. It is kept as a patch in the benchmark harness (`patches/m23-face-culling.patch`, outside the repo) for scenes whose shadowed lights stand behind or beside the camera.
  - M24, the shadow driver (§10):
    - Why: a trace of the moving-shadow benchmark (Chrome tracing, which doubles the frame to 38 ms) put 25.5 ms in Bevy's camera driver. That was 91 `Core3d` runs of about 0.28 ms each, of which the systems doing work took 0.03 ms. Submitting took another 7.9 ms, for 371 command buffers. The GPU time per face was 0.018 ms, about 1.6 ms for all 90.
    - The README has no lit-pass GPU time with moving shadows because Bevy's diagnostics record at most 256 timestamps per frame. The face views render before the camera and used them all, about eight each. The driver keeps one span for all shadow views (`dynlight_shadow_views`) and one for their preprocessing.
    - Images: the shadow scenes render bit-identical frames to the build before (`realtime_shadows`, `lighting_test`, `stress_lights`), and the four shadow captures (`realtime_shadows`, `outdoor`, `outdoor_unbaked`, `outdoor_moving_shadow`) match their baselines exactly. WebGPU in Edge renders 9 of 12 faces per frame, as before. A test checks that the driver's schedule applies its systems' command buffers.
    - Timing (8 alternating runs of e777221 and the driver, medians, RTX 4080 SUPER, Direct3D 12): with 15 of 100 lights redrawing all their faces (90 per frame), the frame drops from 22.6 to 8.6 ms. The shadow views now take 1.77 ms of GPU time, 0.23 ms of it in preprocessing, and the lit pass shows again (0.54 ms). With the scene still, the frame goes from 2.18 to 2.02 ms.
  - M25 dropped: a subgroup fast path for the light loops, committed as a253052 and reverted.
    - What it did: where wgpu has `SUBGROUP`, a subgroup whose pixels shared their realtime span, or their baked triangle, visited it through `subgroupBroadcastFirst` values. The driver could then load each light once per subgroup. The other pixels took the same loop with their own values, so frames were bit-identical with and without. naga 30 rejects `enable subgroups;` as unimplemented but accepts the builtins without it.
    - Why it went: the lit pass was slower on the RTX 4080 in every case. These are 8 alternating runs of one build with the setting on and off, so the draw order matched:
      - 1000 baked lights: 0.746 against 0.696 ms; with a depth prepass, 0.681 against 0.645 ms.
      - 1000 moving realtime lights: 1.631 against 1.565 ms; with a prepass, 1.437 against 1.385 ms.
      - High quality: 1.48 against 1.38 ms. Worse, its pipeline took more than 6 s to compile on Direct3D 12, and lit meshes stay invisible meanwhile.
    - Codegen: each loop is compiled twice, so the shader binary grew by 42% (383 to 543 KB) while the register counts stayed the same. The shared loads saved less than the extra code cost.
    - Kept for later: the patch is in the harness (`patches/m25-subgroups.patch`), for GPUs where scalar loads matter more (AMD). A one-loop version, taking the `subgroupMin` of each pixel's next light, would compile each loop once, but it needs ascending light lists.
  - M26 not done: a depth prepass by default was to land only on a clear win, and it isn't one. These are 10 alternating runs at 6c210f5, medians of the mean frame time and of p95:
    - 1000 baked lights: the frame goes from 1.98 to 2.10 ms (p95 2.38 against 2.37). The lit pass drops from 0.69 to 0.64 ms, but the prepass itself takes 0.07 ms.
    - 1000 moving realtime lights: the frame stays at 2.17 ms. GPU time drops from 1.61 to 1.49 ms, counting the prepass.
    - Still shadows: the frame goes from 1.93 to 2.15 ms, p95 from 2.18 to 2.46 ms.
    - Moving shadows: the frame goes from 8.38 to 8.63 ms, p95 from 9.04 to 9.48 ms.
    - So the prepass stays an opt-in `DepthPrepass` on the camera. It pays only where the lit pass is heavy and the frame waits on the GPU.
  - Found on the way: the harness's baked scene (`bench_1000.dynlight`) was still in format 2 and failed to load at version 4, so its baked runs silently fell back to unbaked meshes. Re-baked, the baked lit pass takes 0.70 ms, against 0.61 ms in the README (2026-10-17). Draw order, which changes between builds, can explain part of that. But the shader also grew with the sun and sky paths (M19–M22): shader-report counts 96 registers for the all-features variant on the RTX 4080, where it counted 80 in round 2.

## 15. Testing

- **CPU mirrors:** lifted from bevyout for attenuation, light types and volumetrics. Compared against GPU readback at chosen pixels (Bevy's `gpu_readback` pattern). This replaces Unity goldens. **Done:** the `gpu_check` example (1024 pixels, four light types, largest difference 0.001), and later `gpu_check_specular`, `gpu_check_area`, `gpu_check_sun` and `gpu_check_sky` (§14).
- **Effects:** determinism, output range, and noise-threshold statistics.
- **Light BVH:** query equals brute force.
- **Triangle builder:** a header/offset round-trip; the shader-side lookup reimplemented in Rust over the serialized words.
- **Bake correctness:** plane + box occluder + light gives a shadow footprint that matches the analytic one. Bakes are deterministic regardless of thread count.
- **Screenshot harness** from bevyout: enabled vs disabled, per-region diffs plus value checks. Cucumber scenarios 1–10.
  - **Done as** `tests/gpu_examples.rs`, ignored by default because hosted runners have no GPU (`cargo test --test gpu_examples -- --ignored`). It runs `gpu_check`, and checks that a `lighting_test` screenshot is lit and not flat, and that the light-count debug view changes it.
  - **Cucumber dropped:** bevyout's scenarios mostly pin Unity parity (frozen upstream defaults, the Unity-compatible effect phase), which §2 forbids. The rest are unit tests here: `light::tests::defaults_match_henrys_light`, `fog::tests::defaults_match_henrys_light`, `effects::tests::{strobe_toggles_once_per_step, every_effect_stays_finite_and_in_range, time_driven_effects_share_the_global_phase}`, `fog::tests::inactive_volumes_are_skipped`, `shared::gpu_light::tests::channel_bits_round_trip`. Adding `cucumber` for duplicates isn't worth the dependency.
- **Visual baselines, done 2026-10-17:** `tests/gpu_baselines.rs` (ignored, needs a GPU) renders eight still views and compares them with `tests/baselines/*.png` using NVIDIA's FLIP (`nv-flip`, a native-only dev-dependency because it compiles C++).
  - The views: `lighting_test` baked (overview, panel, fog, Metallic, light-count view), `lighting_test` unbaked, `realtime_shadows` and `stress_lights` with 100 baked lights. The triangle-ID fallback is checked against the overview baseline; its frame is identical. M19 adds `outdoor` baked and unbaked, and the baked view through FXC; M20 re-blessed both `outdoor` views for the sky, M21 for its baked shadows, and M22 for the sun's realtime shadows, with `outdoor_moving_shadow` new.
  - `--capture` (`examples/support/capture.rs`) fixes the window at 1280×720 at scale 1. It stops virtual time at 2.5 s, which freezes effects, shimmer, the shader clock and animation. It saves once any `--bake` has finished (in memory), no pipeline has compiled for 0.3 s, and two screenshots 0.3 s apart match.
  - Repeated captures are bit-identical. Direct3D 12 against the Vulkan baselines gives a mean FLIP below 0.00001 and a largest error of 0.011 (RTX 4080 SUPER).
  - Limits: mean 0.002, and at most 0.01% of pixels above 0.1. Deliberate regressions that fail: 3% darker albedo (mean 0.016–0.038), and a point-light shadow blur of σ 1 instead of 1.5 (mean 0.0033, but 1.1% of pixels above 0.1).
  - Not caught: halving the panel's bake samples (mean 0.0006, 9 visible pixels; FLIP rates it invisible), and switching off the LTC specular, which no baseline view shows because no rectangle light reaches a Metallic surface. `gpu_check_specular` covers that path.
  - Found on the way, fixed the same day: on Direct3D 12 without `dxcompiler.dll`, wgpu falls back to FXC. Bevy only picks DXC when `dxcompiler.dll` is in the working directory or `WGPU_DX12_COMPILER=dxc` is set. FXC failed to compile the lit fragment shader.
    - The cause: the light BVH walk wrote its node stack, an array inside the `BvhWalk` struct, at a run-time index through a pointer. FXC calls that "not natively addressable" and then fails to unroll the loops around it.
    - The fix: the stack is a separate array (`dynamic_geometry/shaders/light_bvh.wesl`). Vulkan frames are bit-identical. It was first a module-scope `var<private>`, which made the lit pass 3.7× slower on DXC (§14, experiments); now the caller declares it as a function variable and passes it by pointer.
    - On FXC the lit pipeline takes about 12 s to compile. The readback checks therefore wait up to 60 s for a match, and `lighting_test_renders_a_lit_scene` uses `--capture`.
    - Every GPU test passes on Direct3D 12 with FXC forced. The baselines stay within mean FLIP 0.0008 there; the panel differs most. `lighting_test_on_fxc_matches_the_overview` (Windows only) guards it.
    - How it was found: naga's HLSL backend (`hlsl-out`) on the surface WESL modules, with the Windows SDK's `fxc.exe`, reproduces such errors in a fraction of a second without a GPU.
- **Benchmarks:** rays/s, bake time of the sample scene, frame time at 100/500/1000 lights.
  - **Done:** `benches/bake.rs` (`cargo bench --bench bake`; 26–28 M shadow rays/s on a 32-thread desktop CPU, bounce for 64 lights in 3.4 s) and the `stress_lights` example for frame times (README).

## 16. Defaults (easy to change)

- **Targets:** desktop Vulkan, DX12 and Metal first. WebGPU and mobile come through the M9 fallback. No WebGL2: it has no storage buffers in fragment shaders.
- **Baking:** CPU baker (deterministic, headless). A GPU baker via wgpu ray queries can come later.
- **Fidelity:** match Henry's look (formulas, kernels, defaults), not bit-exact Unity output.
- **Light grid:** `realtime_cluster_min_lights` is 8. In the harness the grid made no measurable difference at 2 and 4 realtime lights and saved GPU time from 8 on (M17).
