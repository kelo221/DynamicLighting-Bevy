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
| `effects/` | 15 intensity effects, our noise + RNG, fixed timestep | `BO/core/{runtime,fixed_timestep}.rs`, `C/Core/LightEffects/*` |
| `surface/` | `DynLitMaterial` variants, prepass, BRDF, `light_processor.wesl`, `lighting.wesl` | `S/DynamicLighting{Diffuse,Metallic,Unlit}.shader`, `S/Generators/LightProcessor.cginc` |
| `bake/` | Tracer steps (§8), `.dynlight` format, `BakeRequest` + progress | `C/Core/DynamicLightingTracer*.cs`, `DynamicTrianglesBuilder.cs`, `Acceleration/BvhLightStructure.cs`, `BO/baker.rs` codec |
| `baked_lighting/` | Asset loading, buffers, receiver records, primitive-index path + fallback, `triangles.wesl` | `S/DynamicLighting.cginc:608-962`, `BO/viewer/scene.rs` |
| `dynamic_geometry/` | Light BVH + distance cubes at runtime | `S/DynamicLighting.cginc:358-566,969-1148`, `C/Core/DynamicLightManager.DistanceCubes.cs` |
| `realtime_shadows/` | VSM cube pass + blur | `C/Core/DynamicLightManager.ShadowCamera.cs`, `S/Operations/{ShadowDepth,GaussianBlur}.shader` |
| `cookies/` | Cookie texture array | `C/Core/DynamicLightManager.LightCookie.cs` |
| `fog/` | Volumetric fog pass + CPU mirror | `BO/render/pipeline.rs:149-181,300-362`, `BO/core/volumetric.rs`, `S/DynamicLightingPostProcessing.shader` |
| `gltf_import/` | glTF extras → `DynamicLight`, material swap | new |
| `debug/` | Gizmos, debug views, diagnostics | `C/Lighting/DynamicLight.cs:502-748` (gizmo ideas) |

Repo-level folders: `examples/` (`lighting_test` migrated from bevyout, effects gallery, baked room, dynamic objects), `tests/`, `benches/`, `assets/`, `LICENSES/` (Henry MIT, SmoShaders MIT), and `docs/PORT_MAP.md` (provenance per ported file).

## 4. Authoring API (Bevy side)

Henry's single `DynamicLight` MonoBehaviour is split into optional components, so a light only carries what it uses (implemented in M0):

- **`DynamicLight`** (`light/`, `#[require(Transform, Visibility)]`, `Reflect`):
  - Fields and defaults mirror Henry's `C/Lighting/DynamicLight.cs`: color, intensity 2, radius 4, falloff, `realtime: bool` (Henry's realtime channel), type, shadow mode, illumination, transparency, cutoffs 26°/30°, wave/rotor/disco params, bounce (`LightBounce`: color/modifier/intensity/samples/compression), shimmer + scale + modifier. The cookie handle arrives with the `cookies/` slice in M1.
  - Enums match Henry's: shadow is `Raytraced`/`Realtime`; transparency is `Disabled`/`Enabled`/`EnabledMax`; `Shimmer` is `None`/`Water`/`Random`.
- **`LightEffect`** (`effects/`): kind + pulse speed/modifier/offset + timestep 1/30. It requires `LightEffectState` (current intensity, fixed-timestep accumulator, per-light RNG seeded from `BakeId` or the entity).
- **`VolumetricLight`** (`fog/`): shape (`Sphere`/`Box`/`ConeZ`/`ConeY`), radius, thickness, intensity, visibility. A light without it has no fog.
- **`BakeStatic`** (`bake/`): the static flag. It means "casts and receives baked shadows", like Henry's `isStatic` + `shadowCastingMode != Off`.
- **`BakeId(u64)`:** stable identity for lights and meshes. It is derived from the `Name` path + mesh handle when absent. It replaces Unity's InstanceID ordering and object references, and makes BVH order, mesh mapping and the "light moved from its baked origin" check deterministic.
- **Resources:** Henry's manager settings are split the same way:
  - `DynamicLightingSettings` (`shared/`): ambient, realtime light budget (32), shadow cubemap budget, quality tier, dynamic-geometry mode, tracking mode, global random seed. The volumetric budget and debug view are added with their slices.
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
  - bit 15: shadow cubemap
  - bit 16: cookie
  - bits 17–19: bounce bits-per-pixel − 1
  - bits 20–22: baked shadow bits per texel − 1 (1 for Henry's lights, 4 for area lights; Beyond Henry, §12)
- **Area lights** are one record at the shape's center (`position`); they take no cookies.
- **`gpN` per light type:** follows `C/Core/DynamicLightManager.cs:1322-1420`.
- **Reuse:** lift the ABI, packing and offset tests from `BO/render/gpu.rs`, and check that its field order matches the reordered layout.

**Light buffer contents.**

- Baked lights come first, in light-BVH order (the bake reorders them); realtime lights follow.
- A globals header comes first in the same buffer. It holds the baked count, realtime count, ambient, time (our own clock; Bevy's `globals.time` wraps at 3600 s) and the quality level. That saves a binding, and globals change every frame, which a per-material uniform would make expensive.

**`dynamic_triangles` (u32 words).** Henry's format, unchanged (`C/Core/DynamicTrianglesBuilder.cs:198-366`):

- **Per-triangle header:** `[lightDataOffset, boundsX−2, boundsY−2, W]`.
- **Light block:** `[count, (lightIndex, shadowOffset[, bounceOffset])…]`.
- **Shadow bits:** LSB-first, bit index `x + y·W`. Area lights store 4 bits per texel (§12, Beyond Henry): `32 / bits` texels per word, texel `j` at bit `j · bits`; 1 bit is Henry's layout.
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
- Distance cubes: 64²×6 f16 pairs per baked light (12,288 words each).
- These are merged into one buffer to stay under the per-stage storage-buffer limit.

**Textures.**

- **Distance-cube index lookup:** `texture_cube<u32>`, R32Uint, read with `textureGather`.
- **Cookies:** 2D array.
- **Realtime shadow cubemaps (VSM):**
  - Henry uses a 512² cube array with 16 cubes.
  - `Rg32Float` isn't filterable without the `FLOAT32_FILTERABLE` feature. **Done:** `Rg32Float` when the device has the feature (Bevy requests all adapter features by default), `Rg16Float` otherwise; the render world builds pipelines for both. Variance needs the precision: `Rg16Float` shows acne on caps and crate faces.

**Storage-buffer budget.** We use 3 storage buffers: lights, triangles, aux. Bevy's view/mesh groups use some more, and WebGPU's default is 8 per stage. Measure in M1. If needed, merge triangles into aux, or raise limits on native. Triangle data with bounce can exceed the default 128 MiB binding size, so on native, raise `max_storage_buffer_binding_size`.

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

**Dropped:** ForwardAdd passes, the ShadowCaster pass, Unity lightmaps (`LIGHTMAP_ON`), reflection-probe macros, Unity fog, instancing macros, the legacy transparent shader, and BlitColorDepth.

**Replaced:**

- `Material::enable_shadows() = false`.
- An alpha-cutoff prepass fragment shader.
- An optional own `texture_cube` reflection map for Metallic. Never `EnvironmentMapLight`.

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
12. **Write the `.dynlight` asset.**
    - Container: lift bevyout's codec (magic, version, gzip via `flate2`, a CRC-32 checksum via `crc32fast`).
    - Sections: settings (bounce on, dynamic-geometry mode), lights (`BakeId`, baked origin, compression), BVH nodes, meshes (`BakeId`, content hash, resolution, triangle words), distance cubes, stats.

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
4. **Realtime selection.** Budget 32. If over budget, sort by distance to the active camera, then frustum-test the light spheres (our own sphere-vs-`Frustum` test; the camera frustum isn't lighting).
5. **Pack lights.** Fill `GpuLight`s (packing from `BO/render/gpu.rs`, `gpN` per type, gamma→linear colors, bounce color mix, `falloff = r·falloff²`). Write the `ShaderBuffer` (resizable without rebuilding materials).
6. **Cookies.** Dedupe per texture into the cookie array, budget 64. Copy only when a cookie changes; Henry re-blits every frame.
7. **Realtime VSM shadows** (§10) for `RealtimeShadows` lights within the budget of 16.
8. **Volumetric lights.** Build a separate persistent buffer for the fog pass, without the cap of 2 that bevyout had.
9. **Upload once.** Load-time data (triangles, aux, lookup cube) is uploaded once when the asset loads. ⇒ Fixes bevyout re-uploading the whole bake every frame (`BO/render/gpu.rs:302-334`).
10. **Stale detection.** Mesh or light hash mismatches against the bake fall back to the dynamic path and are reported in diagnostics.

## 10. Render passes (our own, as systems in `Core3d`)

**Realtime VSM shadow cubemaps** (Henry's `DynamicLightManager.ShadowCamera.cs`). Per light: 6 faces at 512², storing moments `(d/r, (d/r)²)`, then a separable Gaussian blur (kernel 3, σ 1) into the cube array.

- **Spike A:** follow Bevy 0.20's `render_depth_to_texture` example.
  - Use depth-only cameras (`RenderTarget::None` + `DepthPrepass`, 90° FOV, one per face).
  - Add our own system after `Core3dSystems::Prepass` that converts depth to linear-distance moments and blurs them.
- **Spike B:** if A's CPU cost per camera is too high, use a custom render phase (`custom_render_phase` example) that draws casters into all faces in one system.
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
- Shadow face cameras are Bevy depth-prepass cameras (`RenderTarget::None`); the moments and the horizontal blur run in one pass, mirrored into the left-handed cube face, then a vertical blur pass writes the array layer.
- Bounce light needs non-overlapping UV1 (one bounce map per mesh, as in Henry's). Bevy's `Cuboid` UV0 overlaps, so the examples build a UV1 atlas.
- `Transparency::EnabledMax` behaves like `Enabled`: our shadow rays alpha-test directly, so there is no photon-cube alternative.
- Directional lights don't exist in Henry's runtime either; `light::sun_light` follows his editor menu (a point light 2.5 km away with a 10 km radius).
- Metallic shading: diffuse isn't divided by π (the specular is scaled by π instead), so a rough dielectric under Metallic shading matches Diffuse shading; specular uses plain Lambert on dynamic geometry.
- Moved baked lights: a light counts as moved beyond 1 mm from its baked origin (Henry: 1e-5 m).
- Shadow face cameras end at the light radius (Henry: 3× the radius). Occluders beyond the radius can't stand between the light and anything it reaches. Cubemaps go to the lights nearest the camera first; Henry hands them out in light-buffer order.
- Transparency: shadow rays alpha-test the texture directly, with the material's own mask cutoff, so Henry's 5×5 alpha filter over photon cubes has no counterpart.
- The light BVH query also tests each light's own sphere, so leaves never return lights out of range.
- Bounce sampling seeds one random stream per texel (not one per sample); the output is still deterministic.
- `TrackingMode` has no effect: transforms are change-tracked in Bevy, so both modes see current positions.

**Beyond Henry** (features his package doesn't have, M10 and M11, and their approximations):

- Shadow cubemaps keep their slot and only redraw faces that changed (`realtime_shadows::slots`): a face renders again when its light moves or changes radius, the array is recreated, the set of casters it draws changes, a `Mesh` or `DynLitMaterial` asset changes, or a caster that moved, appeared, disappeared or is skinned overlaps the face's frustum. Every face also redraws for a few frames after a pass ran with pipelines still compiling. A light standing where it was baked leaves baked meshes out of its cubemap (`realtime_shadows::filter`): their shadows are in the bake already, so its cubemap only holds moving casters.
- Area lights (`LightType::Rectangle`, `LightType::Tube`) are two more light types in Henry's family. Their shading comes from a shape, and their baked shadows break one of Henry's rules: a texel stores how much of the shape it sees, in 4 bits, instead of 1 bit for a single point.
  - Kept: shadows are traced once per texel at bake time over static geometry in a fixed triangle order; the light is one record at its center with Henry's `radius` sphere (light BVH bounds, per-triangle light lists, distance cube and stale detection as for a point light); a baked light that moves, turns or changes its shape falls back to realtime lighting until the next bake (`light::placement`); its color, intensity and effects can change freely (and a tube's unused height). Point and spot lights keep 1 bit and bake the same shadow bytes as before.
  - Baked visibility (`bake::area`): probe rays to a rectangle's corners and center, or a tube's ends and center, settle fully lit and fully shadowed texels. Where they disagree, `BakeSettings::area_light_samples` rays (64 by default, at most 32 in a Preview bake; the lab uses 128) go to points on the shape: an R2 sequence over a rectangle, golden-ratio steps along a tube, offset per texel by a stateless hash of the seed, mesh, light and texel. Each ray is weighted by `(n·d + 0.1)` (Henry's −0.1 NdotL margin), the emitter's cosine and `1 / d²`, and the lit share is rounded to the nearest of 16 levels (`bake::shadow_grid`). Seams and padding treat levels like Henry's bits.
  - Data: shadow words pack `32 / bits` texels (§5); channel bits 20–22 carry the bits per texel. The `.dynlight` format version 2 stores each light's kind, rotation, size and bits per texel (60-byte light records) and rejects version 1. The shaders read a row of eight texels with at most two loads at any depth, so every filter tier costs the same loads as for 1 bit and returns levels over 15 (`baked_lighting::mirror` checks every tier against texel reads at 1, 2 and 4 bits).
  - Moving objects (both cube modes, `dynamic_geometry::mirror`): the distance cube stays at the light's center and now stores exact distances (Henry's +0.25 m `DistanceCubes` bias moved into the shader). A light point offset by `o` sees a blocker a fraction `s` of the way to the receiver where the center sees `d + o · (1 − s) / s`. A blocker search over the shape scaled by 3 finds the mean `s` (each blocker's distance over the receiver plane's along the same ray, exact for blockers parallel to the receiver), then 8 lattice taps over the shape scaled by `(1 − s) / s` give the share of the light that reaches the receiver. Every lookup blends four depth tests bilinearly (Henry's angular test), the filter's on grids of 1, 2, 4, … texel blocks sized to the taps' spacing and blended between the two nearest levels, so penumbras ramp without steps. Plane distances stop at the light's radius, which the cube stores for empty space. Limits: blockers nearer the light than a quarter of the way soften as if they were a quarter of the way; penumbra ends round off by about a tap's spacing (within 0.075 of a traced reference in the tests); a lookup costs 32 to 96 buffer reads per area light per fragment, only on meshes without bake data.
  - `radius` is the sphere around the light's position where the light ends, as for every type. Past its shape the light reaches `radius` minus half the shape's diagonal (`DynamicLight::shape_reach`, packed as the record's radius); a light whose shape doesn't fit inside its radius is invalid and disabled, like a broken spot cone. The bake skips texels farther than that from the shape.
  - Shading (the only new math): diffuse is the mean cosine over the shape (Lambert's polygon form factor for rectangles, the closed-form line integral for tubes, each over the angle the shape covers); a rectangle also has a Lambertian emitter cosine at its nearest point, so it lights only its front. Attenuation uses the distance to the nearest point over the reach past the shape. Specular uses one representative point with Karis's (2013) energy normalization, applied per axis of the rectangle (an approximation of the 2-D widening).
  - Realtime shadow cubemaps are seen from the light's position. Their faces are blurred wider the larger the shape is next to the radius: σ from 1.5 up to 4 texels, reached when half the diagonal is a quarter of the radius (`realtime_shadows::faces::face_blur`, 5 taps a side at σ 4). This softens the shadows of moving objects evenly; it doesn't model a penumbra that grows with the distance between occluder and receiver.
  - Why 4 bits: M12 and M13 kept Henry's 1-bit rule by baking an area light as 3 to 16 point lights. Penumbras came in visible steps (at most 16, wider than the filter tiers blur), and each point cost a light slot, a distance cube, per-triangle list entries and shadow reads. Soft visibility costs 4× a 1-bit light's shadow words, more bake rays only in penumbras, nothing extra at runtime, and gives smooth penumbras that harden where objects touch the ground. Lights on moving objects lose nothing: they never used baked shadows.
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

## 15. Testing

- **CPU mirrors:** lifted from bevyout for attenuation, light types and volumetrics. Compared against GPU readback at chosen pixels (Bevy's `gpu_readback` pattern). This replaces Unity goldens. **Done:** the `gpu_check` example (1024 pixels, four light types, largest difference 0.001).
- **Effects:** determinism, output range, and noise-threshold statistics.
- **Light BVH:** query equals brute force.
- **Triangle builder:** a header/offset round-trip; the shader-side lookup reimplemented in Rust over the serialized words.
- **Bake correctness:** plane + box occluder + light gives a shadow footprint that matches the analytic one. Bakes are deterministic regardless of thread count.
- **Screenshot harness** from bevyout: enabled vs disabled, per-region diffs plus value checks. Cucumber scenarios 1–10.
  - **Done as** `tests/gpu_examples.rs`, ignored by default because hosted runners have no GPU (`cargo test --test gpu_examples -- --ignored`). It runs `gpu_check`, and checks that a `lighting_test` screenshot is lit and not flat, and that the light-count debug view changes it.
  - **Cucumber dropped:** bevyout's scenarios mostly pin Unity parity (frozen upstream defaults, the Unity-compatible effect phase), which §2 forbids. The rest are unit tests here: `light::tests::defaults_match_henrys_light`, `fog::tests::defaults_match_henrys_light`, `effects::tests::{strobe_toggles_once_per_step, every_effect_stays_finite_and_in_range, time_driven_effects_share_the_global_phase}`, `fog::tests::inactive_volumes_are_skipped`, `shared::gpu_light::tests::channel_bits_round_trip`. Adding `cucumber` for duplicates isn't worth the dependency.
- **Benchmarks:** rays/s, bake time of the sample scene, frame time at 100/500/1000 lights.
  - **Done:** `benches/bake.rs` (`cargo bench --bench bake`; 26–28 M shadow rays/s on a 32-thread desktop CPU, bounce for 64 lights in 3.4 s) and the `stress_lights` example for frame times (README).

## 16. Defaults (easy to change)

- **Targets:** desktop Vulkan, DX12 and Metal first. WebGPU and mobile come through the M9 fallback. No WebGL2: it has no storage buffers in fragment shaders.
- **Baking:** CPU baker (deterministic, headless). A GPU baker via wgpu ray queries can come later.
- **Fidelity:** match Henry's look (formulas, kernels, defaults), not bit-exact Unity output.
