# Port map

Provenance for every source file of the crate: one row per `*.rs` / `*.wesl` file under `src/`, `tests/`, `examples/` and `benches/`. Files that are entirely new say so. Code carries no provenance comments; this map is the record (see `AGENTS.md`).

## Sources

| Short | Source | Licence |
|---|---|---|
| `S/` | Henry de Jongh, [DynamicLighting](https://github.com/Henry00IS/DynamicLighting) `AlpacaIT.DynamicLighting/Shaders/`, commit `e2a51ad` | MIT, notice in `LICENSES/DynamicLighting-MIT.txt` |
| `C/` | Same repository, `AlpacaIT.DynamicLighting/Scripts/` | MIT, as above |
| `E/` | Same repository, `AlpacaIT.DynamicLighting.Editor/Editor/` | MIT, as above |
| SmoShaders | Daniel Ilett, [smo-shaders](https://github.com/daniel-ilett/smo-shaders) Gaussian blur, as found in Henry's package: `S/Operations/GaussianBlur.shader` (VSM blur) and `C/Utilities/GuassianBlur.cs` (bounce blur); Henry ships the notice as `AlpacaIT.DynamicLighting/Licenses/SmoShaders.txt` | MIT, © 2019 Daniel Ilett, notice in `LICENSES/SmoShaders-MIT.txt` |
| LTC | Eric Heitz, Jonathan Dupuy, Stephen Hill and David Neubelt, [ltc_code](https://github.com/selfshadow/ltc_code) commit `31e5e96`: the fitted GGX tables `fit/results/ltc_1.dds` and `ltc_2.dds`, from "Real-Time Polygonal-Light Shading with Linearly Transformed Cosines" (SIGGRAPH 2016) | BSD-2-Clause with a request to cite the paper, notice in `LICENSES/LTC-BSD-2.txt` |
| `BO/` | [kelo221/bevyout](https://github.com/kelo221/bevyout) branch `DynamicLightsPort`, `src/vsa/dynamic_lighting/`, commit `f2c7a6a`. Paths outside that folder are written as `bevyout/src/...` | The repository is GPL-3.0; the code lifted here is relicensed by its sole author under this crate's MIT OR Apache-2.0 |

Nothing here is copied from Unity. Where Henry or bevyout relied on Unity behaviour (`UnityEngine.Random`, `Mathf.PerlinNoise`, physics raycasts, camera renders, `ScriptableObject` storage, InstanceIDs, Unity goldens), the replacement is our own code and the "Status" column says so. Henry's code that reproduces third-party snippets (LearnOpenGL BRDF, Shadertoy `rand`) is not ported; see the rows for `brdf.wesl` and `light_types.wesl`.

## Status values

- **Ported:** translated from Henry's code, same formulas.
- **Lifted:** taken from bevyout, adjusted to the slice layout.
- **Replaced:** behaviour Henry got from Unity, re-implemented from scratch.
- **New:** no counterpart.

Notable deviations from Henry follow the status word. `mod.rs` rows hold only wiring and say what they wire.

## Files

### `lib.rs`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/lib.rs` | none | none | New; composes the slice plugins into `DynamicLightingPlugin`, plus a `prelude` |

### `shared/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/shared/mod.rs` | none | none | New; wiring: `SharedPlugin` (settings resource, loads `types.wesl`), `shaders_available`, re-exports |
| `src/shared/bake_id.rs` | (Unity InstanceID ordering) | none | Replaced; FNV-1a over the length-prefixed `Name` path; `ResolvedBakeId` for runtime matching |
| `src/shared/cube.rs` | `S/Internal/Common.cginc:43-94`; `C/Utilities/PhotonCube.cs:213-275`; `C/Core/DynamicLightManager.DistanceCubes.cs:37-81` | none | Replaced; face order and (face, uv) ↔ direction re-derived from the Vulkan/WebGPU cube face selection table, not copied |
| `src/shared/globals.rs` | `S/DynamicLighting.cginc:343-356`; `C/Core/DynamicLightManager.Shaders.cs:227-268` | none | New; 48-byte header at the start of the light buffer (counts, ambient, our own clock instead of `_Time`, quality, flags, and the distant-light counts of M19); the `FLAG_DEBUG_*` bits for the debug views are New |
| `src/shared/feature_choice.rs` | keyword selection at scene load, `C/Core/DynamicLightManager.cs` (`UpdateShaderKeywords`) | none | New; chooses the `ShaderFeatures` of `FeatureSelection::Automatic` once the scene stops loading (PLAN §7.3) |
| `src/shared/feature_use.rs` | none | none | New; the features the scene uses, the ones the lit shader gets, and the warning for a feature in use while switched off |
| `src/shared/gpu_light.rs` | `C/Core/ShaderDynamicLight.cs:1-89`; channel bits `C/Core/DynamicLightManager.cs:1283-1313` | `BO/render/gpu.rs:30-220` | Lifted; flags renamed, `bounce_color` before `falloff` (112 bytes), layout test checks the WESL struct through naga; channel bits 20–22 hold the baked shadow bits per texel (New, PLAN §12) |
| `src/shared/math.rs` | `Mathf.Lerp`/`InverseLerp`/`MoveTowards`/`SmoothStep` call sites | `BO/core/unity_math.rs:18-46` | Replaced; helpers named for what they do |
| `src/shared/mesh_hash.rs` | (Unity `Mesh.GetFastHash`, `C/Core/DynamicLightManager.cs:646,1208`) | none | New; hashes triangle corners (position + UV1 bits) in draw order, so de-indexed meshes keep their hash |
| `src/shared/name_path.rs` | none | none | New; `Name` paths through the hierarchy, used instead of entity ids for anything saved or rendered |
| `src/shared/settings.rs` | `C/Core/DynamicLightingSettings.cs`; manager fields `C/Core/DynamicLightManager.cs:198-310`; ambient `C/Core/DynamicLightManager.Shaders.cs:249-268`; `C/Lighting/DynamicLightingRuntimeQuality.cs`, `DynamicGeometryLightingMode.cs`, `DynamicLightTrackingMode.cs`; lighting-only view `E/Interface/DrawModes/SceneViewDrawModeLighting.cs` | `BO/core/config.rs` | Lifted; the `DebugView` enum is New (its `Lighting` view follows Henry's draw mode, `LightCount` has no counterpart); `TrackingMode` is kept for compatibility but has no effect (Bevy transforms are change-tracked, so positions are always current); `realtime_cluster_min_lights` is New (`light_clusters`) |
| `src/shared/shader_buffer.rs` | none | none | New; rewrites a `ShaderBuffer` asset in place, keeping its GPU size |
| `src/shared/sky.rs` | none | none | New; the sky's 144-byte spherical-harmonic block after the globals header (M20), with its layout test against the WESL |
| `src/shared/shaders/types.wesl` | `S/DynamicLighting.cginc:10-130` (struct, type/shimmer constants, flag getters); globals `:343-356` | `BO/shaders/dynamic_lighting_types.wgsl` | Ported to WESL; adds the `LightGlobals` header and the `DynamicLights` buffer struct; `light_group`, the `SkyIrradiance` block and `LIGHT_TYPE_SKY` (New) |
| `src/shared/variant.rs` | Henry's shader keywords, `C/Core/DynamicLightManager.cs` | none | New; generates the `variant` WESL module (`QUALITY`, `FEATURE_*` constants) under a fixed handle (PLAN §7.3) |
| `src/shared/wesl_check.rs` | none | none | New (test-only); compiles WESL with Bevy's `wesl` and inspects layouts with naga |

### `effects/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/effects/mod.rs` | none | none | New; wiring: `EffectsPlugin`, `EffectsSystems` set in `PostUpdate`, re-exports |
| `src/effects/advance.rs` | `C/Core/DynamicLightManager.cs:1445-1541` (`UpdateLightEffects`) | `BO/core/runtime.rs` | Ported |
| `src/effects/continuous.rs` | `C/Core/LightEffects/*.cs` (11 files) | `BO/core/runtime.rs` | Ported; same formulas, our noise |
| `src/effects/effect.rs` | `C/Lighting/DynamicLightEffect.cs`; effect fields of `C/Lighting/DynamicLight.cs` | `BO/core/types.rs`, `BO/core/config.rs` | Lifted; split out of the light component |
| `src/effects/fixed_timestep.rs` | `C/Utilities/MathEx.cs:353-412` (`FixedTimestep`); timestep field `C/Lighting/DynamicLight.cs:307-320`; use `C/Core/DynamicLightManager.cs:1043,1074` | `BO/core/fixed_timestep.rs` | Lifted |
| `src/effects/noise.rs` | (`Mathf.PerlinNoise`) | replaces Perlin in `BO/core/unity_math.rs` | Replaced; own gradient noise, statistically calibrated |
| `src/effects/rng.rs` | (`UnityEngine.Random`) | replaces `BO/core/unity_random.rs` | Replaced; PCG32 seeded by SplitMix64, one stream per light |
| `src/effects/systems.rs` | `C/Core/DynamicLightManager.cs:1110,1133` | `BO/bevy_bridge/mod.rs` | Lifted |
| `src/effects/tests.rs` | none | none (the Unity-parity tests in `BO/core/runtime.rs:297-401` are dropped) | New |

### `light/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/light/mod.rs` | none | none | New; wiring: `LightPlugin`, `LightSystems::{Attach, Pack}`, loads `light_types.wesl`, `ltc.wesl` and `area.wesl`, creates the light buffer and the LTC table image |
| `src/light/ltc.rs` | none | none | New; CPU reference of `ltc.wesl` and `area_ltc_specular`, written from the LTC paper; reads the authors' table (LTC source) like a linear sampler |
| `src/light/ltc_lut.bin` | none | none | Data: LTC source `ltc_1.dds` and `ltc_2.dds` without their 128-byte DDS headers, layer 0 first |
| `src/light/ltc_image.rs` | none | none | New; the table as a two-layer RGBA16F array image under `AREA_LIGHT_LUT` |
| `src/light/ltc_tests.rs` | none | none | New; the table, the quad integral against Lambert and the transformed lobe, the fit against brute-force GGX, and `rectangle_specular` |
| `src/light/area.rs` | none | none | New; CPU reference of `area.wesl` (nearest point, mean cosine, facing cosine for the realtime-shadow terminator, representative point); area lights are Henry-family types shaded from their shape, one record at its center, with soft baked shadows (PLAN §12, Beyond Henry) |
| `src/light/area_record_tests.rs` | none | none | New; packed area-light records: their shape, nothing lit past the radius, and shadow-point records adding up to the whole light |
| `src/light/area_tests.rs` | none | none | New; the area-light math against brute-force integration over the shapes |
| `src/light/budget.rs` | `C/Core/DynamicLightManager.cs:1051-1088,1258-1262` | none | Ported; stable sort (ties keep order); Henry's editor-only frustum bypass dropped |
| `src/light/buffer.rs` | `C/Core/DynamicLightManager.cs:903-924,1154-1159` | none (replaces the per-frame buffers of `BO/render/gpu.rs:246-470`) | New; globals header + sky irradiance + baked slots + realtime lights in one `ShaderBuffer`; also records the camera the realtime lights were chosen for (`RealtimeViewer`), which `light_clusters` builds its grid over |
| `src/light/component.rs` | `C/Lighting/DynamicLight.cs:30-370` (fields, defaults) | `BO/core/config.rs` | Lifted |
| `src/light/features.rs` | none | none | New; which `ShaderFeatures` the packed lights need |
| `src/light/kinds.rs` | `C/Lighting/DynamicLight{Type,ShadowMode,IlluminationMode,TransparencyMode,Shimmer}.cs`, `C/Lighting/DynamicBounceLightingCompressionMode.cs` | `BO/core/types.rs` | Lifted; `Sun` and `is_distant` (M19) and `Sky` (M20) are New; enums match Henry's; `EnabledMax` behaves as `Enabled` (rays alpha-test directly) |
| `src/light/mirror.rs` | `S/DynamicLighting.cginc:140-336` | `BO/core/spatial.rs` | Ported; right-handed light space; equal cutoffs give a hard edge (PLAN §12) |
| `src/light/pack.rs` | `C/Core/DynamicLightManager.cs:1264-1443` (`SetShaderDynamicLight`) | `BO/render/gpu.rs:85-161` and its tests `:530-665` | Lifted; spot `forward` from Bevy's −Z forward; invalid cones disabled; area-light half extents, the reach past the shape and the shadow bits per texel are New |
| `src/light/placement.rs` | moved baked lights `C/Core/DynamicLightManager.cs:1004-1159` | none | Ported, split out of `systems.rs`: the `BakedInPlace` marker read by packing and the shadow cubemaps; moved threshold 1 mm (Henry: `ApproximatelyEquals`, 1e-5); area lights also need the rotation and shape they were baked with, suns only their direction and disk (New) |
| `src/light/presets.rs` | `E/EditorMenus.cs:119-167` (point, spot, directional, discoball menu items) | none | Ported; `sun_light` now makes a `LightType::Sun` (M19, PLAN §12) where Henry's directional item; `DynamicLight::sky()` is New (M20) |
| `src/light/shape.rs` | none | none | New; `LightShape`, what a light's baked shadows were traced for beyond its position: an area light's shape, a sun's direction and disk (`SunDisk`), or the sky, which may turn freely (M21) |
| `src/light/shader_tests.rs` | none | none | New |
| `src/light/shaders/area.wesl` | none | none | New; written from the published formulas: Lambert's polygon form factor, the line form factor, Karis 2013's representative point and energy normalization, and `area_ltc_specular` (rectangles, M15) |
| `src/light/shaders/ltc.wesl` | none | none | New; written from the LTC paper (Heitz et al. 2016): shading frame, inverse transform, horizon clipping and the polygon integral with the authors' fit of θ / sin θ |
| `src/light/shaders/sun.wesl` | none | none | New; the sun's disk for specular light, mirrored by `light::sun` |
| `src/light/shaders/light_types.wesl` | `S/DynamicLighting.cginc:140-341`; `S/Internal/Common.cginc:3-37`; `S/Generators/TrilinearFilter.cginc:1-36` | `BO/shaders/dynamic_lighting_spatial.wgsl`, `BO/shaders/dynamic_lighting_common.wgsl` | Lifted; shimmer uses the lowbias32 hash (public domain) instead of Shadertoy `rand` (`Common.cginc:39-41`); our clock instead of `_Time`; right = up × forward |
| `src/light/spatial.rs` | `C/Core/DynamicLightManager.cs:1320-1407`; cookie scale `:1345` | `BO/core/spatial.rs` | Lifted; Unity-parity tests dropped; the cookie scale clamps the cone to 0–90° |
| `src/light/sun.rs` | none | none | New; CPU reference of `sun.wesl`: the specular direction within the sun's disk and Karis's energy normalization (M19, PLAN §12) |
| `src/light/systems.rs` | `C/Core/DynamicLightManager.cs:1004-1159` (moved baked lights, budget, packing order); `C/Core/DynamicLightManager.LightPositions.cs:67-100` | none | Ported; moved-light detection lives in `placement.rs` |
| `src/light/tests.rs` | none | none | New |

### `light_clusters/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/light_clusters/mod.rs` | none | none | New; wiring: `LightClustersPlugin`, `LightClustersSystems` after `LightSystems::Pack`, loads `clusters.wesl` and creates the cluster buffer |
| `src/light_clusters/grid.rs` | none | none | New; the 16 × 9 × 24 cell grid over the realtime-selection camera (exponential depth slices, linear for orthographic cameras), the cell of a world position (CPU mirror of `realtime_span`) and the cells a sphere's view-space box covers (PLAN §12, Beyond Henry) |
| `src/light_clusters/assign.rs` | none | none | New; light spheres (area lights add their extent), per-cell lists in light order packed as two u16 per word, and the reasons the grid stays off |
| `src/light_clusters/bevy_clusters.rs` | none | none | New; keeps Bevy's own light clustering on the CPU in a single cell, since dynlight uses no Bevy lights but every draw binds Bevy's view group |
| `src/light_clusters/gpu.rs` | none | none | New; `GpuClusterHeader` and its layout tests against the WESL |
| `src/light_clusters/buffer.rs` | none | none | New; the `REALTIME_CLUSTERS` `ShaderBuffer` |
| `src/light_clusters/systems.rs` | none | none | New; builds the lists from the packed realtime lights and uploads them each frame |
| `src/light_clusters/shaders/clusters.wesl` | none | none | New; binding 29, `realtime_span` (a fragment's cell list, or every realtime light outside the grid) and `span_light` |
| `src/light_clusters/shader_tests.rs` | none | none | New |
| `src/light_clusters/tests.rs` | none | none | New; every light whose sphere holds a random point is listed in its cell (perspective and orthographic), list lengths, fallbacks, buffer layout |

### `surface/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/surface/mod.rs` | none | none | New; wiring: `SurfacePlugin`, `MaterialPlugin<DynLitMaterial>`, loads the surface WESL modules, alpha-mask system |
| `src/surface/alpha_mask.rs` | `S/Generators/PhotonCubeTransparent.cginc:55-71` (0.5 alpha cutoff) | none | New; derives `BakeAlphaMask` from `DynLitMaterial`; `Mask(c)` materials bake with their own cutoff |
| `src/surface/material.rs` | properties of `S/DynamicLightingDiffuse.shader:3-16`, `S/DynamicLightingMetallic.shader:5-25`, `S/DynamicLightingUnlit.shader:3-14` | none (bevyout's patched `pbr_fragment.wgsl` hook is not used) | New; `Material` on Bevy's mesh pipeline, feature flags in a uniform instead of keywords, `enable_shadows() = false`; the realtime light clusters at binding 29 and the sky's specular cube at 30–31 (New) |
| `src/surface/shader_tests.rs` | none | none | New |
| `src/surface/mirror.rs` | none | none | New; CPU reference of `brdf.wesl` and Metallic shading's per-light specular term in `light_processor.wesl` |
| `src/surface/shared_arrays.rs` | none | none | New; re-prepares lit materials when the shadow cubemap, cookie array or sky cube is replaced |
| `src/surface/unbaked.rs` | none | none | New; reports lit meshes without bake data, for the `unbaked_meshes` shader feature |
| `src/surface/shaders/bindings.wesl` | globals `S/DynamicLighting.cginc:343-356` | none | New; every resource on the material bind group (0–19 material, 20+ scene-wide, 27–28 the LTC table, 29 the realtime light clusters, declared by `clusters.wesl`, 30–31 the sky's specular cube) |
| `src/surface/shaders/brdf.wesl` | none (Henry's `S/Internal/Common.cginc:291-342` is LearnOpenGL code, CC BY-NC) | none | Replaced; GGX / Smith / Schlick rewritten from the published papers |
| `src/surface/shaders/dynlit.wesl` | fragment bodies `S/DynamicLightingDiffuse.shader:110-161`, `S/DynamicLightingMetallic.shader:141-260`; lighting-only mode `S/DynamicLightingDiffuse.shader:141` | none | Ported; `TRIANGLE_ID` / `PRIMITIVE_INDEX` paths; Unity fog and lightmaps dropped |
| `src/surface/shaders/dynlit_prepass.wesl` | `S/Generators/DepthNormals.cginc:40-47`, `S/Generators/DepthOnly.cginc` | none | Ported; alpha cutout only |
| `src/surface/shaders/dynlit_vertex.wesl` | none (Henry relies on `SV_PrimitiveID`) | none | New; Bevy's static mesh vertex shader plus a flat triangle ID |
| `src/surface/shaders/light_processor.wesl` | `S/Generators/LightProcessor.cginc:1-235`; bounce cones `S/DynamicLighting.cginc:154-163,195-207`; per-light terms `S/DynamicLightingDiffuse.shader:163-203`, `S/DynamicLightingMetallic.shader:262-345` | none | Ported; Metallic intensity-applied-twice and bounce cross-terms fixed (PLAN §12); Metallic diffuse not divided by π, specular scaled by π instead; rectangle lights reflect through the LTC table (New, PLAN §12) |
| `src/surface/shaders/lighting.wesl` | `S/DynamicLighting.cginc:1006-1158` (`DYNLIT_FRAGMENT_INTERNAL`) | none | Ported; macros become functions; baked and realtime lights in separate loops; baked entries ≥ `baked_count` ignored; baked shadows decoded at the light's bits per texel, and area lights on dynamic geometry use the soft cube lookup (New, PLAN §12); realtime lights come from the fragment's light-grid cell (New, PLAN §12); realtime distant lights go to the sun or the sky (New) |
| `src/surface/shaders/sky_light.wesl` | none | none | New; `add_sky`: the sky's spherical-harmonic diffuse light, and with Metallic shading its prefiltered reflection weighted by Schlick's Fresnel with roughness (M20, PLAN §12); both scaled by the baked sky visibility (M21) |
| `src/surface/shaders/shade.wesl` | `S/DynamicLightingDiffuse.shader:110-161`, `S/DynamicLightingMetallic.shader:141-260` | none | Ported; Unity reflection probes replaced by an optional own cubemap; the light-count heat map (debug view) is New |

### `bake/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/bake/mod.rs` | none | none | New; wiring: `BakePlugin`, `BakeSystems`, re-exports, `.dynlight` loader registration |
| `src/bake/asset.rs` | `C/Lighting/RaycastedScene.cs:15-159` (the Unity asset holding the bake) | none | Replaced; Bevy `AssetLoader` for `.dynlight` |
| `src/bake/baker.rs` | `C/Core/DynamicLightingTracer.cs:133-352` (`CreateDynamicLightsBvh`, `StartRaytracing`) | none | Ported; one pure function on rayon, Henry's step order; channel assignment (`DynamicLightingTracer.Channels.cs`) skipped (PLAN §8); an area light's points are ordered together for the light BVH; distant lights go last, suns then skies, and a sky adds the probe grid (M21) |
| `src/bake/bits.rs` | `C/Utilities/BitArray2.cs` (bit layout) | none | Ported; adds an OR-only atomic grid so results don't depend on thread order |
| `src/bake/area.rs` | none | none | New; soft visibility of a texel to a rectangle or tube light: probe rays and a minimum of spread samples, then the rest of the samples where they disagree, weighted like Henry's NdotL margin (PLAN §12, Beyond Henry) |
| `src/bake/light_shape.rs` | none | none | New; a baked light's reach, shadow bits per texel and facing test by shape (point, area light, sun, sky at 8 bits), with Henry's −0.1 NdotL tolerance |
| `src/bake/bounce.rs` | `C/Core/DynamicLightingTracer.cs:719-929` (`BounceTriangle`); photon cubes `C/Core/DynamicLightingTracer.PhotonCamera.cs:133-181`, `S/Operations/PhotonCube.shader:46-70`, `C/Utilities/PhotonCube.cs:277-365` | none | Ported gather; photon cubes Replaced by CPU closest-hit rays (default 512², Henry 1024²); our RNG for `onUnitSphere`; at least 2 samples (Henry divides by `samples − 1`) |
| `src/bake/bounce_map.rs` | `C/Core/DynamicLightingTracer.cs:410-465,1150-1246`; blur `C/Utilities/GuassianBlur.cs:10-102` (SmoShaders) | none | Ported; keeps only a window around the crops; the blur reads from a copy (Henry's passes alias source and destination) |
| `src/bake/bounce_pack.rs` | `C/Core/DynamicTrianglesBuilder.cs:368-599` (`CompressBounceLighting*`) | none (`pack_quantized`, `BO/baker.rs:533-553`, is not used: no sqrt, no dither) | Ported; dither from our PCG32 instead of `Random.Range` |
| `src/bake/distance_cubes.rs` | `C/Core/DynamicLightingTracer.DistanceCubes.cs:10-71` | none | Replaced; CPU closest-hit rays instead of photon camera renders, WebGPU face order; f16 pairs kept; exact distances (Henry's +0.25 m bias moved into the shader, PLAN §12) |
| `src/bake/format/mod.rs` | (Henry: gzip-compressed `uint` arrays in a Unity `ScriptableObject`, `C/Lighting/RaycastedScene.cs:19-159`) | gzip container idea `BO/baker.rs:118-183` | New; `.dynlight` header with magic, format version (4 since M21: light kinds and shadow bits, sun maps, sky probes) and CRC-32 of the payload (bevyout: SHA-256 of the file); light records and the sun-map and sky-probe sections are in `lights.rs` and `maps.rs` |
| `src/bake/format/lights.rs` | none | none | New; the 60-byte light records, their kinds (point-like, rectangle, tube, sun, sky) and order checks (M21, split out of `mod.rs`) |
| `src/bake/format/maps.rs` | none | none | New; the sun-map and sky-probe sections and their validation (M21, split out of `mod.rs`) |
| `src/bake/format/codec.rs` | none | `BO/baker.rs:668-716` (`put_*`, `ByteReader`) | Lifted; u64 counts, rejects lengths the data can't hold |
| `src/bake/format/tests.rs` | none | none | New; round trip, unknown versions, corruption, light order, sun-map and sky-probe validation |
| `src/bake/format/types.rs` | `C/Lighting/RaycastedScene.cs:108-120`, `C/Lighting/RaycastedMeshRenderer.cs`, `C/Lighting/RaycastedDynamicLight.cs` | none | Replaced; plain structs keyed by `BakeId`; `SkyProbes` and its unorm8 packing are New (M21) |
| `src/bake/geometry.rs` | `C/Utilities/MathEx.cs:15-35,59-197,327-348` | none | Ported; map size clamped to ≥ 2; `approx_eq` fixes `ApproximatelyEquals` comparing absolute values |
| `src/bake/identity.rs` | (Unity InstanceID ordering and object references) | none | Replaced; `BakeId` from entity `Name` paths (+ mesh asset path), collisions resolved deterministically |
| `src/bake/input.rs` | what gets baked `C/Core/DynamicLightingTracer.TemporaryScene.cs:43-118`; UV1 with UV0 fallback `C/Utilities/MeshBuilder.cs:33-110` | none (replaces the inputs of `BO/baker.rs:20-117`) | New; plain data, no ECS types; area lights carry their shape (`AreaLightShape`); `area_light_samples`, `area_light_min_samples`, `sky_samples`, `sky_probe_spacing` and the seed per bake |
| `src/bake/light_bvh.rs` | `C/Core/Acceleration/BvhLightStructure.cs:1-326`, `C/Core/Acceleration/BvhLightNode.cs:1-38` | none | Ported; the query also tests each light's bounds, so it equals brute force |
| `src/bake/mesh_bake.rs` | `C/Core/DynamicLightingTracer.cs:354-717` (`Raytrace`, `RaycastTriangle`) | none | Ported; rayon over triangles instead of `RaycastCommand` batches; −0.1 NdotL and 0.001 offset kept; area lights trace soft visibility per texel (New, PLAN §12) |
| `src/bake/texel_shadow.rs` | `C/Core/DynamicLightingTracer.cs:354-717` (the per-texel ray of `RaycastTriangle`) | none | Ported, split out of `mesh_bake.rs`; dispatches on the light's shape: Henry's single ray, area-light, sun or sky visibility |
| `src/bake/mesh_builder.rs` | `C/Utilities/MeshBuilder.cs:1-148`; rects `C/Core/DynamicTrianglesBuilder.cs:75-99`, `C/Core/DynamicLightingTracer.cs:940-958,1008-1026`, `C/Utilities/PixelTriangleRect.cs:36-60` | none | Ported; trace and optimize rects derived from texel centers (Henry scales UVs by `size − 1` there) |
| `src/bake/mirror.rs` | `S/DynamicLighting.cginc:384-392,608-962` | none | Ported (test-only CPU mirror of the `triangles.wesl` layout reads, shadow levels at any bits per texel, and the cube fetch; the shadow filters are mirrored in `baked_lighting/mirror.rs`) |
| `src/bake/occluder.rs` | Unity physics: `C/Core/DynamicLightingTracer.Jobs.cs`, `C/Core/Acceleration/Raycast*.cs`; transparency `C/Utilities/PhotonCube.cs:367-457`, `S/Generators/PhotonCubeTransparent.cginc:55-71` | none | Replaced; `obvhs` CWBVH (built from its PLOC, reinsertion and CWBVH steps, untimed for wasm), two-sided, `t ≤ max`, −0 direction components made +0 (obvhs's node test misses those rays); transparency alpha-tested during traversal instead of photon cubes (no 5×5 alpha filter) |
| `src/bake/progress.rs` | (Unity progress bar, `C/Core/DynamicLightingTracer.cs:215-275,374-420`) | none | Replaced; lock-free counters and a cancel flag |
| `src/bake/random.rs` | (`UnityEngine.Random.onUnitSphere`, `C/Core/DynamicLightingTracer.cs:719-724`; `Random.Range`, `C/Core/DynamicTrianglesBuilder.cs:450-575`) | none | Replaced; stateless keyed seeds into our PCG32 |
| `src/bake/seams.rs` | `C/Core/DynamicLightingTracer.SeamsPaddingStep.cs:1-168`; `DilateShadowBits` `C/Core/DynamicLightingTracer.cs:1080-1148` | none | Ported; the row wrap at x = 0 is fixed (PLAN §12) |
| `src/bake/settings.rs` | `C/Core/DynamicLightManager.cs:255-263` (pixel density); max size `:1581`; `C/Lighting/DynamicBounceLightingDefaultCompressionMode.cs` | `BO/core/config.rs` | Lifted; `BakeStatic` replaces Henry's static flag + shadow casting check; the area-light, sun-map and sky settings are New |
| `src/bake/shadows.rs` | `C/Core/DynamicLightingTracer.cs:931-1078` (`OptimizeTriangle`, `BuildShadows`) | none | Ported; parallel per triangle |
| `src/bake/shadow_grid.rs` | `C/Utilities/BitArray2.cs` (layout at 1 bit) | none | New; shadow grids at 1, 2 or 4 bits per texel, an atomic grid that only raises levels so results don't depend on thread order |
| `src/bake/spread.rs` | none | none | New; area-light sample points: the Sobol (0, 2) sequence over rectangles, van der Corput's along tubes, shifted per texel (PLAN §12, Beyond Henry) |
| `src/bake/sky.rs` | none | none | New; how much of the sky a texel sees: cosine-weighted rays over its hemisphere through the Sobol spread (Malley's method), stored in 8 bits (M21, PLAN §12) |
| `src/bake/sky_probes.rs` | none | none | New; the sky-probe grid over the static bounds: six hemisphere shares per probe, probes inside geometry filled from their neighbours (M21, PLAN §12) |
| `src/bake/sun.rs` | none | none | New; soft visibility of a texel to a sun's disk: probes at its center and rim, then the Sobol spread through the concentric disk mapping (M19, PLAN §12) |
| `src/bake/sun_map.rs` | none | none | New; the orthographic unorm16 depth map of the static scene along each baked sun's rays, for meshes without bake data (M19) |
| `src/bake/snapshot.rs` | `C/Core/DynamicLightingTracer.TemporaryScene.cs:43-118`; `C/Core/DynamicLightManager.cs:522-548` | none | Replaced; ECS query (`BakeStatic` meshes, non-realtime lights); area lights with their shape (New); sky lights bake unless `realtime` (M21; M20 left them out) |
| `src/bake/systems.rs` | `E/EditorMenus.cs:9-67` (bake menus); `C/Core/DynamicLightManager.cs:1581-1621` | none | Replaced; `BakeRequest` message on the async compute pool, Preview / Full presets; in-memory result, optional file |
| `src/bake/texels.rs` | `C/Core/DynamicLightingTracer.TriangleUvToFull3dStep.cs:1-225`; `C/Utilities/MathEx.cs:185-273` | none | Ported; explicit "outside" result and renormalized normals (PLAN §12) |
| `src/bake/triangle_words.rs` | `C/Core/DynamicTrianglesBuilder.cs:75-99,198-366` | none (the `BO/baker.rs` serializer is not reused) | Ported; same word layout |
| `src/bake/tests/mod.rs` | none | none | New; synthetic floor, cuboid and light fixtures |
| `src/bake/tests/bounce.rs` | none | none | New |
| `src/bake/tests/determinism.rs` | none | none | New |
| `src/bake/tests/sun.rs` | none | none | New; hard sun shadows as the parallel projection, penumbras that widen with the disk and the blocker height, suns outside the light BVH, bounce and distance cubes |
| `src/bake/tests/sky.rs` | none | none | New; roofed against open floors, a wall's analytic view factor, probes under a roof and inside geometry (M21) |
| `src/bake/tests/footprint.rs` | none | none | New; analytic shadow footprint |
| `src/bake/tests/layout.rs` | none | none | New |
| `src/bake/tests/light_bvh.rs` | none | none | New; moved out of `light_bvh.rs`: Henry's leaves, queries against brute force, area lights kept together |
| `src/bake/tests/occluder.rs` | none | none | New |

### `baked_lighting/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/baked_lighting/mod.rs` | none | none | New; wiring: `BakedLightingPlugin`, `PostUpdate` set, loads `baked_data` / `triangles` WESL |
| `src/baked_lighting/apply.rs` | `C/Core/DynamicLightManager.cs:550-749` (`Initialize`: buffers per renderer, BVH, distance cubes, baked light slots) | none | Replaced; `ActiveBake` resource, applied again when the asset reloads |
| `src/baked_lighting/features.rs` | none | none | New; detects wgpu `PRIMITIVE_INDEX` (Henry relies on `SV_PrimitiveID`) |
| `src/baked_lighting/layout.rs` | buffers `S/DynamicLighting.cginc:343-382,589-606,1003` | none | New; one triangles buffer and one aux buffer (header, receivers, BVH nodes, distance cubes, sun maps, sky probes); per-mesh offsets stay relative as in Henry |
| `src/baked_lighting/mirror.rs` | `S/DynamicLighting.cginc:608-962` (`shadow_sample_*`) | none | New; CPU mirror of the four shadow filter tiers at 1, 2, 4 and 8 bits per texel; its tests, against texel-by-texel reads, are in `mirror_tests.rs` |
| `src/baked_lighting/mirror_tests.rs` | none | none | New; every filter tier against texel reads at 1, 2, 4 and 8 bits, split out of `mirror.rs` (M21) |
| `src/baked_lighting/shader_tests.rs` | none | none | New; naga validation and constants against `layout.rs` and the mirror |
| `src/baked_lighting/shaders/baked_data.wesl` | `S/DynamicLighting.cginc:343-348,606` | none | New; receiver lookup by `MeshTag` instead of per-renderer `lightmap_resolution` |
| `src/baked_lighting/shaders/triangles.wesl` | `S/DynamicLighting.cginc:589-962` (`DynamicTriangle`) | none | Ported; u32 wrap math kept; realtime lights loop separately, so the stale-offset quirk can't occur (PLAN §12); shadow texels read eight to a row (at most two loads) at 1, 2 or 4 bits per texel and four at 8 bits (two row reads for the Gaussian), the 5×5 Gaussian applied across then down; rows above the grid repeat its first row (PLAN §12); levels scale to 0–1 (New) |
| `src/baked_lighting/systems.rs` | stale detection `C/Core/DynamicLightManager.cs:1167-1230` | none (replaces `bevyout/src/viewer/scene.rs:34-101`, `MeshTag` from glTF extras and spawn order) | Replaced; stable tags from `BakeId` + content hash; a mismatch falls back to the unbaked path and warns once |
| `src/baked_lighting/triangle_ids.rs` | none | none | New; de-indexed meshes with a triangle-ID attribute for GPUs without `primitive_index` |

### `dynamic_geometry/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/dynamic_geometry/mod.rs` | none | none | New; wiring: loads `light_bvh` / `distance_cubes` / `sun_map` / `sky_probes` WESL |
| `src/dynamic_geometry/mirror.rs`, `mirror_tests.rs` | `S/DynamicLighting.cginc:468-565` (the angular depth test it builds on) | none | New; CPU mirror of `cube_area_shadow`, tested against a traced share of the light, for steps, and for empty space past the radius |
| `src/dynamic_geometry/sun_mirror.rs` | none | none | New; CPU mirror of `sun_map.wesl`, the sun map lookup with the baked shadows' 4×4 filter shape |
| `src/dynamic_geometry/sky_mirror.rs` | none | none | New; CPU mirror of `sky_probes.wesl`, the trilinear probe lookup weighted like an ambient cube (M21) |
| `src/dynamic_geometry/shader_tests.rs` | none | none | New |
| `src/dynamic_geometry/shaders/distance_cubes.wesl` | `S/DynamicLighting.cginc:384-565`; index lookup cube `C/Core/DynamicLightManager.DistanceCubes.cs:37-81` | none | Ported; faces re-derived for WebGPU, texel index computed instead of the R32 lookup cube; the +0.25 m bias added here. Henry's in-file copyright header for the angular filter (`S/DynamicLighting.cginc:468-469`) is kept verbatim above `cube_angular`. `cube_area_shadow` (soft lookup for area lights, built on his angular depth test) is New (PLAN §12) |
| `src/dynamic_geometry/shaders/sun_map.wesl` | none | none | New; shadows of baked suns on meshes without bake data from the sun map, the sun's counterpart of a distance cube (M19, PLAN §12) |
| `src/dynamic_geometry/shaders/sky_probes.wesl` | none | none | New; baked sky shadows on meshes without bake data from the sky probes, blended trilinearly and weighted by the normal (M21, PLAN §12) |
| `src/dynamic_geometry/shaders/light_bvh.wesl` | `S/DynamicLighting.cginc:969-1001,1048-1117` | none | Ported; u32 index stack instead of a struct stack, iterator-style API |

### `realtime_shadows/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/realtime_shadows/mod.rs` | none | none | New; wiring: `RealtimeShadowsPlugin`, the face phase and its draw commands, extraction, queue and `Core3d` systems, loads `vsm` / `shadow_kernel` WESL |
| `src/realtime_shadows/casters.rs` | none | none | New; pure caster lists per face from bounding spheres (default render layer, hidden meshes out, meshes without bounds in every face, baked meshes out of cubes whose light is at its baked position) |
| `src/realtime_shadows/cubemaps.rs` | `C/Core/DynamicLightManager.ShadowCamera.cs:49-50,61-63,99` | none | Ported; Rg32Float when the device has `FLOAT32_FILTERABLE`, Rg16Float otherwise; the array grows up to the budget |
| `src/realtime_shadows/extract.rs` | none | none | New; each rendering face becomes a render-world view (`ExtractedView`, `RootNonCameraView`, `NoIndirectDrawing`) with its casters; faces that stop rendering lose it |
| `src/realtime_shadows/face_tests.rs` | none | none | New; app tests: listed casters are marked visible without a camera, and only rendering faces carry a view |
| `src/realtime_shadows/faces.rs` | `C/Core/DynamicLightManager.ShadowCamera.cs:11-19,49` | none | Replaced; cube faces re-derived for WebGPU (left-handed faces, mirrored writes); nearest lights first (Henry: light-buffer order); wider face blur for area lights (New) |
| `src/realtime_shadows/materials.rs` | none | none | New; `CasterMaterial` / `CasterMaterialPlugin`: casters that switch material, whose material is edited or whose textures (re)load redraw the faces that see them |
| `src/realtime_shadows/phase.rs` | none | none | New; the binned `ShadowFace3d` phase and `Reported`, which wraps Bevy's prepass draw commands and reports faces whose draws skip |
| `src/realtime_shadows/queue.rs` | none | none | New; specializes each caster through its material's `prepass_specialize` with Bevy's depth-prepass key (opaque and masked cast) and fills a fresh phase per rendering face |
| `src/realtime_shadows/render.rs` | `C/Core/DynamicLightManager.ShadowCamera.cs:189-201` (two blur blits per face) | none | Replaced; our `Core3d` system per face view: casters into a shared depth texture, then the moments and blur passes, both binding the face uniform for the blur width |
| `src/realtime_shadows/mirror.rs` | `S/Generators/LightProcessor.cginc:122-136`; `S/Internal/Common.cginc:280-289` | none | New; CPU reference of `vsm.wesl` (terminator, Chebyshev test, bleeding cut) and of the face blur kernel, with a blurred-sphere test |
| `src/realtime_shadows/shader_tests.rs` | none | none | New; also checks the VSM and blur constants and the face uniform against `mirror` |
| `src/realtime_shadows/slots.rs` | none | none | New; stable cube slots per light and per-face dirty tracking; a changed blur width redraws the cube; faces reported incomplete redraw alone |
| `src/realtime_shadows/shaders/shadow_blur.wesl` | `S/Operations/GaussianBlur.shader:70-94` (vertical pass, SmoShaders) | none | Ported; the kernel's taps (5 for point lights); writes the face's cube array layer |
| `src/realtime_shadows/shaders/shadow_kernel.wesl` | `S/Operations/GaussianBlur.shader:19-30` (kernel 3, spread 1, SmoShaders) | none | Ported, widened to 5 taps at σ = 1.5 (PLAN §12), and up to 11 taps at σ 4 for area lights (New); weights computed from σ and normalized over the taps |
| `src/realtime_shadows/shaders/shadow_moments.wesl` | `S/Operations/ShadowDepth.shader:42-46`; `S/Operations/GaussianBlur.shader:41-65` (horizontal pass, SmoShaders) | none | Ported; distance moments reconstructed from the reverse-Z depth prepass, fused with the horizontal blur |
| `src/realtime_shadows/shaders/vsm.wesl` | `S/Generators/LightProcessor.cginc:122-136`; `S/Internal/Common.cginc:280-289` (`linstep`, `ReduceLightBleeding`) | none | Ported; NaN guard on the variance (PLAN §7.2); a smooth terminator on the true cosine (for area lights, to the point of the shape the surface faces most) replaces the `1 − NdotL` bleeding term (PLAN §12) |
| `src/realtime_shadows/tests.rs` | none | none | New; caster tracking in an app: material, texture and render-layer edits redraw, unrelated asset events don't, and only real moves do (not a transform set to its old value; also for casters without bounds) |
| `src/realtime_shadows/tracking.rs` | none | none | New; shadow casters that moved (transform compared with the last one seen), appeared, disappeared, animate, or changed render layers, mesh or material, and incomplete renderings (whole passes, or single faces by array layer) |
| `src/realtime_shadows/views.rs` | `C/Core/DynamicLightManager.ShadowCamera.cs:54-108,152-206` | none (bevyout's `shadow_proxy.rs` is dropped) | Replaced; six face entities per light (90° views from `PerspectiveProjection`, not cameras) instead of Unity camera renders; far plane = radius (Henry: 3 × radius); none at IntegratedGraphics quality; only faces that changed render (Henry renders every face every frame); the face blur width follows the light's shape; lists each face's casters and marks them visible |

### `cookies/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/cookies/mod.rs` | `C/Core/DynamicLightManager.LightCookie.cs:44-91` | none | Ported; `LightCookie` component, slots per image; the array is rebuilt only when cookies change (Henry re-blits every frame); no frustum test; Henry's 0°/over-90° guard (`:62`) ported; layers of images no light uses are reused once the budget is full |
| `src/cookies/layers.rs` | `C/Core/DynamicLightManager.LightCookie.cs:14-31` (1024², budget 64) | none | Replaced; CPU resample to R8 layers instead of `Graphics.Blit` into an RHalf array |

### `environment/`

Henry's package has no sky light; the whole slice is New (M20, PLAN §12). The formulas come from the published literature: Ramamoorthi and Hanrahan 2001 for spherical-harmonic irradiance, Karis 2013 for the split-sum GGX prefilter, and Křivánek and Colbert 2008 for its filtered importance sampling.

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/environment/mod.rs` | none | none | New; wiring: `EnvironmentPlugin`, `SkyEnvironment` (requires a `DynamicLight::sky()`), `NoSkyBackground`, `SkyStatus`, the `SKY_SPECULAR` and `SKY_BACKGROUND` handles, loads `sky.wesl` and `background.wesl` |
| `src/environment/equirect.rs` | none | none | New; equirect directions in Blender's mapping, bilinear sampling that wraps in u, solid-angle-weighted mip pyramid, the radiance clamp |
| `src/environment/sh.rs` | none | none | New; nine-coefficient projection weighted by texel solid angle, summed in row order, and the irradiance over π |
| `src/environment/prefilter.rs` | none | none | New; the 128² specular cube, one roughness per level, 64 Hammersley GGX samples per texel read from the pyramid at the sample's solid angle |
| `src/environment/images.rs` | none | none | New; equirect images from 32-bit, 16-bit and 8-bit RGBA, the Rgba16Float cube and background images, placeholders |
| `src/environment/frame.rs` | none | none | New; `SkyFrame`, the sky's turn and tint from its packed light record, and `first_sky`, which looks in the baked and the realtime distant slots |
| `src/environment/systems.rs` | none | none | New; builds a sky's data off the main thread, rebuilds it when the image changes, warns about extra skies |
| `src/environment/render.rs` | none | none | New; the background pass in `Core3d` between the opaque and transparent passes, behind opaque geometry at depth 0 |
| `src/environment/mirror.rs` | none | none | New; CPU reference of `sky.wesl`, `background.wesl` and `add_sky` |
| `src/environment/shader_tests.rs` | none | none | New |
| `src/environment/tests.rs` | none | none | New; the sky build and rebuild in a headless app, and the clamp limiting the light but not the background |
| `src/environment/shaders/sky.wesl` | none | none | New; the sky frame, irradiance from the coefficients, equirect UVs and the background's level of detail |
| `src/environment/shaders/background.wesl` | none | none | New; full-screen triangle that draws the sky image where no geometry was drawn |

### `fog/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/fog/mod.rs` | none | none | New; wiring: `FogPlugin`, loads `fog_volumes` WESL, render-world setup |
| `src/fog/mirror.rs` | `S/DynamicLightingPostProcessing.shader:78-183` | `BO/core/volumetric.rs` | Ported; the turning box and the `Frustum` shaft (a ray clipped against six planes) are New |
| `src/fog/params.rs` | `C/Core/DynamicLightManager.PostProcessing.cs:52-123`; `C/Core/DynamicLightManager.cs:1421-1437` | `BO/core/volumetric.rs`, `BO/shaders/dynamic_lighting_volumetric.wgsl` (record layout) | Lifted; `Frustum` packing and bounds are New |
| `src/fog/render.rs` | `C/Lighting/DynamicLightingPostProcessing.cs:11-46` | `BO/render/pipeline.rs:149-181,300-362` | Lifted; runs after tonemapping (display-space blend like Henry's image effect); MSAA depth supported |
| `src/fog/shader_tests.rs` | none | none | New |
| `src/fog/shaders/fog.wesl` | `S/DynamicLightingPostProcessing.shader:71-80` | `BO/shaders/dynamic_lighting_volumetric.wgsl:37-69,227-249` | Lifted; depth reconstruction for reverse-Z with an infinite far plane |
| `src/fog/shaders/fog_volumes.wesl` | `S/DynamicLightingPostProcessing.shader:81-185`; `S/Internal/Common.cginc:108-243` | `BO/shaders/dynamic_lighting_volumetric.wgsl:23-36,70-226` | Lifted; reuses the `GpuLight` record; Henry's composite kept (PLAN §12); the box turns with the light and the `Frustum` shaft is New (`ray_shaft`) |
| `src/fog/tests.rs` | none | none (the parity tests in `BO/core/volumetric.rs:431-507` are dropped) | New |
| `src/fog/volumes.rs` | `C/Core/DynamicLightManager.PostProcessing.cs:52-123` | `BO/render/extract.rs:109-130`, `BO/render/gpu.rs:163-221` | Lifted; bevyout's cap of two volumes removed; own persistent buffer |
| `src/fog/volumetric.rs` | volumetric fields of `C/Lighting/DynamicLight.cs:323-370`; `C/Lighting/DynamicLightVolumetricType.cs` | `BO/core/volumetric.rs` | Lifted; split out of the light component; `VolumetricShape::Frustum` is New |

### `gltf_import/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/gltf_import/mod.rs` | none | none | New; wiring: the glTF plugin and its settings |
| `src/gltf_import/convert.rs` | none | none (bevyout's `viewer/material_shading.rs` is not used) | New; `StandardMaterial` → `DynLitMaterial`, Metallic shading where needed |
| `src/gltf_import/extras.rs` | field names and defaults of `C/Lighting/DynamicLight.cs:30-370` | none | New; `dynlight_*` node extras (Blender custom properties); area-light `dynlight_type`/`dynlight_size` and `fog: "frustum"` are New |

### `debug/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `src/debug/mod.rs` | none | none | New; wiring: `DebugPlugin`, `LightGizmos` toggle (the debug views are `shared::DebugView`, drawn by the surface shaders) |
| `src/debug/gizmos.rs` | `C/Lighting/DynamicLight.cs:502-672` (gizmo ideas) | none | New; Bevy gizmos, including area-light shapes |

### Examples and tests

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `examples/gpu_check.rs` | none | none | New; GPU readback compared with `light::mirror` (PLAN §15), including a rectangle and a tube light, with the light grid forced on; `--many` reads 200 small lights back through a perspective camera, so most samples take their lights from cell lists; `--webgpu-limits` caps the device at WebGPU's default limits |
| `examples/gpu_check_area.rs` | none | none | New; a baked and an unbaked floor under a panel and a tube, read back and compared with `baked_lighting::mirror` and `dynamic_geometry::mirror` at every quality tier |
| `examples/gpu_check_sun.rs` | none | none | New; a baked and an unbaked floor under a soft sun and a uniform sky, read back and compared with `baked_lighting::mirror`, `dynamic_geometry::sun_mirror` and `sky_mirror` |
| `examples/gpu_check_sky.rs` | none | none | New; a Diffuse and a Metallic ball under a turned, tinted sky with the background behind them, read back and compared with `environment::mirror` |
| `examples/gpu_check_specular.rs` | none | none | New; a Metallic copper floor at two roughnesses under rectangles, a tube and a point light, read back and compared with `surface::mirror` and `light::ltc` |
| `examples/inspector/main.rs` | role of `E/Inspectors/DynamicLightEditor.cs`; light from `DynamicLight::spot()` (`E/EditorMenus.cs:125-131`) | none | New; Feathers UI, bake button |
| `examples/inspector/panel.rs` | role of `E/Inspectors/DynamicLightEditor.cs` | none | New |
| `examples/lighting_test/main.rs` | none | `bevyout/src/viewer/lighting_test.rs:366-733` (type, effect and fog grids), `:910-919` (timed exit) | Lifted scene idea, rewritten for `DynLitMaterial`; `--frames`, `--screenshot`, `--gizmos` |
| `examples/lighting_test/bake.rs` | `E/EditorMenus.cs:9-67` (bake and preview menus) | none | New; `--bake`, B key, `--preview`, `--triangle-ids` |
| `examples/lighting_test/debug.rs` | `E/Interface/DrawModes/SceneViewDrawModeLighting.cs:1-34` (the lighting view's counterpart) | none | New; `--debug-view lighting\|light-count` and the V key cycle `DynamicLightingSettings::debug_view` |
| `examples/lighting_test/fixtures.rs` | none | fog volumes as in the `main.rs` row | New; the fog volumes, a fluorescent ceiling panel with a fog shaft over two props, and a wall tube |
| `examples/lighting_test/player.rs` | none | none | New; the drivable cube (WASD or arrows by key position, Shift, Space for its realtime-shadowed lantern) |
| `examples/realtime_shadows.rs` | none | none | New; `--still` and the cube faces rendered per frame |
| `examples/stress_lights.rs` | none | none | New; 1000-light benchmark (PLAN §14, M9); 256 drifting realtime lights in a browser |
| `examples/support/mod.rs` | none | none | New; page canvas, status line and query-string arguments for the web demos |
| `examples/support/baking.rs` | none | none | New; in-memory bakes, queued until the browser has painted the status |
| `examples/support/orbit.rs` | none | none | New; drag-to-orbit, scroll-to-zoom camera that drifts when idle and can follow an entity |
| `examples/support/capture.rs` | none | none | New; `--capture`: one still, repeatable 1280×720 frame for the visual baselines, taken once bakes and sky builds have finished |
| `examples/support/lightmap_uvs.rs` | none | none | New; UV1 sets for Bevy's primitive meshes (a per-face atlas for boxes, moved from `lighting_test`) |
| `examples/outdoor/main.rs`, `scene.rs` | none | none | New; a courtyard under a baked soft sun and a baked sky with a porch, a lamp, two Metallic balls, and moving objects shadowed by the sun map and the sky probes |
| `examples/outdoor/sky.rs` | none | none | New; a procedural sky image matched to the sun, or an HDRI from `--hdri`, scaled and turned to the sun |
| `tests/area_light_bake.rs` | none | none | New; panels bake soft shadows close to a brute-force trace, tubes soften only along their length, point lights keep one bit |
| `tests/bake_api.rs` | none | none | New; synthetic fixtures only |
| `tests/bake_ecs.rs` | none | none | New; synthetic fixtures only |
| `tests/gpu_examples.rs` | none | `bevyout/tests/dynamic_lighting_gpu.rs` (approach only) | New; ignored-by-default GPU tests that run the `gpu_check` (also with `--many` at WebGPU's default limits), `gpu_check_specular`, `gpu_check_area`, `gpu_check_sun` and `gpu_check_sky` examples and check `lighting_test` screenshots |
| `tests/gpu_baselines.rs` | none | none | New; visual baselines: still frames of the examples compared with `tests/baselines/` by FLIP |
| `tests/common/mod.rs` | none | none | New; helpers shared by the GPU test files |
| `tests/headless_app.rs` | none | none | New |
| `tests/repo_hygiene.rs` | none | none | New; enforces the `AGENTS.md` rules |

### `benches/`

| Our file | Henry | bevyout | Status |
|---|---|---|---|
| `benches/bake.rs` | none | none | New; synthetic bake benchmark (`harness = false`): without bounce, with bounce, 4× lights |
| `benches/clusters.rs` | none | none | New; light grid build time for 100, 1000 and 10 000 scattered point lights (`harness = false`) |
