# wgpu Capabilities & Limitations Audit

**Scope**: authoritative, project-grounded audit of what `wgpu 29` (our current pinned version) can and cannot do, so future rendering features can be designed against a known capability matrix instead of ad-hoc research per issue.

**Related issue**: [#238](https://github.com/lobinuxsoft/oh_my_engine/issues/238).
**Pinned version**: `wgpu = "29"` (workspace `Cargo.toml`).
**Target hardware**: AMD RDNA 4 (RX 9070 XT) on Linux Bazzite/RADV primary; Windows and Steam Deck RDNA 2 as secondary targets.
**Status as of this revision**: **partial** — Areas A, E, F complete. Areas B, C, D, G, H tracked as follow-up research issues (see bottom of doc).

## Status legend

| Tag | Meaning |
|---|---|
| **supported** | Works in `wgpu 29` out of the box on our target backends (Vulkan + DX12 primary, Metal secondary). No feature flag required, or the required flag is stable. |
| **experimental** | Exposed behind an `EXPERIMENTAL_*` feature flag; API may shift; known bugs tracked upstream. Usable with caution. |
| **workaround** | The desired capability is not a first-class API but can be built from lower-level primitives already shipped. |
| **blocked** | Not available in `wgpu 29` and no accepted proposal in sight. Requires either waiting upstream or leaving `wgpu`. |

All claims are sourced. When a claim could not be verified against a primary source (wgpu release notes, gfx-rs tracker, WebGPU/WGSL spec), it is marked `unverified`.

---

## A. Sky / environment

### A.1 Cubemap sampling
- **Status**: supported
- **Notes**: `TextureViewDimension::Cube` and `TextureViewDimension::CubeArray` are defined in `wgpu-types` v29 (`wgpu-types/src/texture.rs`), mapping to WGSL `texture_cube` / `texture_cube_array`. Cubemaps are created as 2D textures with `depth_or_array_layers: 6` and a view with `dimension: Some(TextureViewDimension::Cube)` — confirmed by the official `examples/features/src/skybox/mod.rs` at tag `v29.0.0`. Mipmap and anisotropic filtering configure via the standard `SamplerDescriptor`; the WebGPU spec defines `GPUTextureViewDimension "cube"` / `"cube-array"` with the same filtering rules as 2D textures. `CubeArray` requires `maxTextureArrayLayers >= 256` by default and has no additional feature gate.
- **Source**: [TextureViewDimension enum @ v29.0.0](https://github.com/gfx-rs/wgpu/blob/v29.0.0/wgpu-types/src/texture.rs), [skybox example @ v29.0.0](https://github.com/gfx-rs/wgpu/blob/v29.0.0/examples/features/src/skybox/mod.rs), [WebGPU spec — GPUTextureViewDimension](https://www.w3.org/TR/webgpu/#enumdef-gputextureviewdimension). RDNA 4 / RADV caveats for cubemap sampling on gfx1201: `unverified` beyond wgpu's compliance test suite coverage.

### A.2 HDR texture formats
- **Status**: supported (with feature gates for some uses)
- **Notes**: `TextureFormat` in v29.0.0 includes `Rgba16Float`, `Rgba32Float`, `Rg11b10Ufloat`, and `Rgb9e5Ufloat`. The `guaranteed_format_features` table (`wgpu-types/src/texture/format.rs` lines 949–990) declares:
  - `Rgba16Float`: MSAA resolve + storage read/write-only + all usages (COPY, TEXTURE_BINDING, RENDER_ATTACHMENT, STORAGE_BINDING). Filterable by default.
  - `Rgba32Float`: storage read/write-only + all usages, but **not MSAA and not filterable** by spec. Filterable requires `Features::FLOAT32_FILTERABLE`; blendable requires `Features::FLOAT32_BLENDABLE` (newly consolidated in v29 on Vulkan + Metal, via PRs #8963 / #9032).
  - `Rg11b10Ufloat`: base MSAA + basic usages; as render target requires `Features::RG11B10UFLOAT_RENDERABLE` (then promoted to MSAA_RESOLVE + attachment).
  - `Rgb9e5Ufloat`: `TEXTURE_BINDING` + COPY only; **no render target, no storage, no MSAA** (format.rs line 1011). Spec-locked read-only.
- **Source**: [format.rs @ v29.0.0 lines 949–990](https://github.com/gfx-rs/wgpu/blob/v29.0.0/wgpu-types/src/texture/format.rs#L949-L990), [features.rs @ v29.0.0 — FLOAT32_BLENDABLE, FLOAT32_FILTERABLE, RG11B10UFLOAT_RENDERABLE](https://github.com/gfx-rs/wgpu/blob/v29.0.0/wgpu-types/src/features.rs), [wgpu v29.0.0 release notes — "Added Features::FLOAT32_BLENDABLE on Vulkan and Metal"](https://github.com/gfx-rs/wgpu/releases/tag/v29.0.0), [WebGPU spec — Texture Format Capabilities](https://www.w3.org/TR/webgpu/#texture-format-caps).

### A.3 Equirectangular → cubemap
- **Status**: supported (workaround required because of no built-in helper)
- **Notes**: Two viable paths, neither automatic:
  - **Compute**: create a storage texture with `depth_or_array_layers: 6`, bind as `texture_storage_2d_array<rgba16float, write>`, dispatch per face sampling the equirect with `textureSampleLevel`. No API friction.
  - **Render-to-cube**: 6 pipelines/passes targeting each face view (see A.4). More verbose but works for render-target-only formats.
  There is no built-in helper (`wgpu::util::*` only exposes `TextureBlitter`, `StagingBelt`, `DeviceExt`, `spirv::*` — see `wgpu/src/util/mod.rs` @ v29.0.0). The conversion shader must be written by us.
- **Source**: [wgpu util mod.rs @ v29.0.0](https://github.com/gfx-rs/wgpu/blob/v29.0.0/wgpu/src/util/mod.rs), [WebGPU spec — Storage Texture Binding](https://www.w3.org/TR/webgpu/#storage-texture), [WGSL spec §9.3.2 texture_storage](https://www.w3.org/TR/WGSL/#texture-storage).

### A.4 Render-to-cubemap / layered rendering
- **Status**: workaround (6 passes mandatory on all native backends)
- **Notes**: WebGPU does **not** expose `gl_Layer` / `SV_RenderTargetArrayIndex` for a single-pass multi-layer emit. wgpu 29 has `Features::MULTIVIEW` (features.rs bit 26) enabling `@builtin(view_index)` in vertex / mesh shaders, but its canonical use is VR — `examples/features/src/multiview/mod.rs` at v29.0.0 is explicit: "commonly used for VR rendering". MULTIVIEW is not a general-purpose layered-rendering substitute and is not natively supported across all backends for arbitrary 6-face cube output. The standard approach in wgpu 29 remains creating 6 `TextureView`s with `base_array_layer: 0..5` and invoking `begin_render_pass` six times. There are no geometry shaders in WebGPU/WGSL — a deliberate spec design.
  - `EXPERIMENTAL_MESH_SHADER_MULTIVIEW` (bit 50) allows mesh-shader multiview, but still view-index not arbitrary layer-index.
- **Source**: [features.rs @ v29.0.0 — MULTIVIEW / EXPERIMENTAL_MESH_SHADER_MULTIVIEW](https://github.com/gfx-rs/wgpu/blob/v29.0.0/wgpu-types/src/features.rs), [multiview example @ v29.0.0](https://github.com/gfx-rs/wgpu/blob/v29.0.0/examples/features/src/multiview/mod.rs), [WebGPU spec — Render Pass Encoder](https://www.w3.org/TR/webgpu/#render-pass-encoder), [WGSL spec — built-in values](https://www.w3.org/TR/WGSL/#builtin-values).

### A.5 Environment map prefiltering
- **Status**: workaround (no built-in utility)
- **Notes**: wgpu 29 exposes no high-level `generate_mipmaps(texture)`. The only relevant `wgpu::util` helper is `TextureBlitter` (`wgpu/src/util/texture_blitter.rs` @ v29.0.0), which blits a view to another view using an internal pipeline — suitable for simple per-mip downsampling, but **not** GGX convolution or split-sum prefiltering. The canonical pattern is `examples/features/src/mipmap/mod.rs` @ v29.0.0, which generates mipmaps by hand with a render-pass chain. For IBL prefiltering (radiance convolution per roughness level + irradiance SH) we must author our own compute shaders; this is a one-time-per-HDRI cost at load, so operationally cheap.
- **Source**: [texture_blitter.rs @ v29.0.0](https://github.com/gfx-rs/wgpu/blob/v29.0.0/wgpu/src/util/texture_blitter.rs), [mipmap example @ v29.0.0](https://github.com/gfx-rs/wgpu/blob/v29.0.0/examples/features/src/mipmap/mod.rs), [wgpu util mod.rs @ v29.0.0](https://github.com/gfx-rs/wgpu/blob/v29.0.0/wgpu/src/util/mod.rs).

### Area A summary
- **Feasible today**:
  - Cubemap + cube-array sampling with mipmap and anisotropic filtering across all three target platforms (RDNA 4 / RDNA 2 / Windows).
  - HDR color buffer in `Rgba16Float` with no extra feature gate (blendable + filterable + MSAA-resolve guaranteed by spec).
  - `Rgb9e5Ufloat` as a read-only source texture for compressed HDRIs — good bandwidth trade-off at load.
  - Equirect → cubemap via compute or render pass with 6 explicit faces.
  - `Features::FLOAT32_BLENDABLE` on Vulkan + Metal from v29 if high-precision accumulation is ever needed.
- **Requires workaround**:
  - Single-pass render-to-cubemap: 6 render passes required; wgpu 29 does not expose a layer-output builtin. Irrelevant for a sky pre-bake.
  - `Rg11b10Ufloat` as render target: gated by `Features::RG11B10UFLOAT_RENDERABLE`; check at runtime (RDNA 4 / RADV typically supports it, `unverified` specifically for gfx1201 until `adapter.features()` is queried on the user's rig).
  - Mipmap / IBL prefiltering: no official helper; implement ad-hoc compute shaders using the `mipmap` example as reference.
  - `Rgba32Float` filterable / blendable: feature-gated (`FLOAT32_FILTERABLE`, `FLOAT32_BLENDABLE`).
- **Blocked**:
  - Arbitrary `gl_Layer`-style layered rendering in a single draw. No geometry shaders in WebGPU. MULTIVIEW is VR view-index, not a general substitute.
  - `Rgb9e5Ufloat` as render target or storage — spec-locked read-only.
- **Project-specific SkyRenderer recommendation**: use `Rgba16Float` as the canonical format for both the runtime cubemap and its prefiltered mip chain (spec-guaranteed MSAA-resolve + storage + filtering), load HDRIs as `Rgb9e5Ufloat` equirect (read-only sampling), and convert to cube via a **single compute pass** emitting into `texture_storage_2d_array<rgba16float, write>` with 6 layers. Avoids 6 render passes, depends on no optional feature, and stays compatible with RDNA 2 (Steam Deck) as well as RDNA 4.

---

## B. Deferred pipeline / multi-pass

> **Status: research pending.** Tracked as follow-up; see end of doc.

Planned investigation: MRT max attachments on RDNA 4 / Steam Deck, depth sampling in a later pass (`Depth32Float` + shader read), storage textures with atomic ops for OIT-style effects.

---

## C. Compute shaders

> **Status: research pending.** Tracked as follow-up; see end of doc.

Planned investigation: workgroup size limits on RDNA 4 / RADV, subgroup (wave) intrinsics in wgpu 29, compute → fragment interop without CPU round-trip, indirect dispatch.

---

## D. Volumetric / 3D textures

> **Status: research pending.** Tracked as follow-up; see end of doc.

Planned investigation: 3D storage textures for volumetric clouds / fog, 3D sampled textures for offline-baked volumes, arrays of 3D volumes.

---

## E. Hardware-accelerated ray tracing

### E.1 Current wgpu 29 state
- **Status**: experimental — ray queries (inline RT) shipping; ray-tracing pipelines still in development.
- **Notes**: wgpu 29 exposes `Features::EXPERIMENTAL_RAY_QUERY`. This unlocks `Device::create_blas` / `create_tlas`, BLAS compaction (`prepare_compaction_async` → `Queue::compact_blas`), 24-bit custom instance data in TLAS, and ray-query intrinsics in WGSL (`rayQueryInitialize`, `rayQueryProceed`, `rayQueryGetCommittedIntersection`, `getCommittedHitVertexPositions`, etc.). WGSL must declare `enable wgpu_ray_query;` (and `wgpu_ray_query_vertex_return;` for vertex fetch). The spec doc explicitly warns: "may contain major bugs". SBT record offset returns `0` (reserved). `@any_hit` shaders cannot call `traceRay()`. Ray-tracing **pipelines** (miss / closest-hit / any-hit shader tables) are NOT yet in the public v29 API surface.
- **Source**: [`docs/api-specs/ray_tracing.md` @ v29](https://github.com/gfx-rs/wgpu/blob/v29/docs/api-specs/ray_tracing.md), [`Features` @ docs.rs](https://docs.rs/wgpu/latest/wgpu/struct.Features.html).

### E.2 Tracker state
- Authoritative tracker: [gfx-rs/wgpu#6762 "Ray Tracing Tracking Issue"](https://github.com/gfx-rs/wgpu/issues/6762), opened 2024-12-16, **open** as of April 2026. Supersedes the original [#1040](https://github.com/gfx-rs/wgpu/issues/1040) (closed as not planned, redirected to #6762).
- Backend coverage for **ray query + acceleration structures**: Vulkan ✅, DX12 ✅ (via [PR #6777](https://github.com/gfx-rs/wgpu/pull/6777)), Metal ✅ (via [PR #8071](https://github.com/gfx-rs/wgpu/pull/8071)). Binding arrays and AS limits merged. ~12 known bugs: AMD GPU test failures ([#6727](https://github.com/gfx-rs/wgpu/issues/6727)), Mesa RADV segfaults, Vulkan alignment, Metal sync, UB in ray queries ([#6761](https://github.com/gfx-rs/wgpu/issues/6761)).
- Pending on tracker: custom AABB intersections, micromap support, partitioned TLASes, `instance_id` → `instance_index` rename, ray-tracing pipeline basic design. Open blocker: [#8560 "Should Metal have ray tracing pipelines?"](https://github.com/gfx-rs/wgpu/issues/8560) — unresolved design question gating cross-backend pipeline work. Earlier pipeline work ([PR #3607](https://github.com/gfx-rs/wgpu/pull/3607)) closed-unmerged after dependency drift.

### E.3 naga / WGSL
- naga **parses and emits** ray-query constructs end-to-end: SPIR-V, HLSL (35 ray-query intrinsics), MSL backends. Gated by `enable wgpu_ray_query` / `enable wgpu_ray_query_vertex_return` WGSL extension directives (wgpu-specific extension identifiers, not the GLSL `rayQueryEXT` spelling).
- naga does **not** yet have IR for ray-tracing-pipeline shader stages (raygen / miss / closest-hit / any-hit / intersection) nor SBT indexing. That lack is one of the gating items on #6762 for pipeline support.
- **Source**: [ray_tracing.md @ trunk](https://github.com/gfx-rs/wgpu/blob/trunk/docs/api-specs/ray_tracing.md), tracker [#6762](https://github.com/gfx-rs/wgpu/issues/6762).

### E.4 Realistic timeline
- **12 months (≈ April 2027)**: Ray-query path stabilizes, the `EXPERIMENTAL_` prefix likely dropped, bugs closed, possibly custom AABB intersections land. Ray-tracing pipelines probably still absent on at least Metal (design question #8560 still open with no consensus). **Not** expected to be non-experimental on DX12 until parity bugs close.
- **24 months (≈ April 2028)**: Ray-tracing pipelines plausible on Vulkan + DX12 if a maintainer picks up the Metal question. No milestone commitment from maintainers. WebGPU-standard RT still blocked: [gpuweb/gpuweb#535](https://github.com/gpuweb/gpuweb/issues/535) assigned to "Milestone 4+" (open, undated) — web side will trail native by years. Any 2028 delivery is contributor-driven, not roadmap-driven.
- **Source**: tracker [#6762](https://github.com/gfx-rs/wgpu/issues/6762), design [#8560](https://github.com/gfx-rs/wgpu/issues/8560), WebGPU [#535](https://github.com/gpuweb/gpuweb/issues/535).

### E.5 Alternatives

#### ash (raw Vulkan)
- Mature, actively maintained, thin bindings. `VK_KHR_acceleration_structure` + `VK_KHR_ray_tracing_pipeline` + `VK_KHR_ray_query` all exposed.
- Cost: manual descriptor management, manual sync, lose cross-platform (Linux + Windows only — the project's stated targets, so acceptable; Metal and browser are cut).
- Reasonable path for "RT pass only" because `wgpu-hal` already depends on `ash` internally. `wgpu::Instance` → `ash::Instance` unwrap is doable via `wgpu_hal::vulkan` exposure APIs plus `Device::as_hal::<vulkan::Api>`.

#### vulkano
- Exposes `vulkano::pipeline::ray_tracing`. However, pre-1.0 with regularly breaking APIs, and its RT-pipeline PR history shows prolonged contributor gaps. Maintenance adequate for hobby, risky for a long-lived engine. Not recommended when `ash` solves the same problem with a thinner, more stable surface.

#### Hybrid (wgpu + ash)
- Technically realistic: `wgpu_hal::vulkan::Device::texture_from_raw` and `Device::as_hal::<vulkan::Api>` let you import a `VkImage` created by `ash` (or vice versa) and share timeline semaphores. Queue family and image layout transitions are the gotcha — the RT pass must emit the barrier wgpu expects on re-entry.
- Known public exemplars are thin: the most-cited community reference is the [wgpu v28 RT tutorial (Zenn)](https://zenn.dev/kokutoupan/articles/eefc517ac4210d?locale=en), which now points at the built-in ray query rather than hybrid. Bevy's [Solari @ 0.17](https://jms55.github.io/posts/2025-09-20-solari-bevy-0-17/) stayed on upstream wgpu ray query instead of going hybrid — strong signal that ray query is "good enough" for real-time GI today and hybrid is not widely validated.
- Recommendation: adopt hybrid only if pipeline-based RT (SBT, recursion, any-hit shaders) is specifically required. For ray-query-only workloads (GI probes, AO, shadow rays, SDF replacement), stay on wgpu 29.

### Area E summary
- **Feasible today** on wgpu 29 (RX 9070 XT + Steam Deck RDNA 2):
  - Inline ray queries against BLAS/TLAS (Vulkan + DX12).
  - BLAS compaction, vertex return, binding arrays of AS.
  - Replacing the SDF ray-marched primary-visibility loop with a real BVH trace is within scope.
- **Blocked**:
  - Ray-tracing pipelines (raygen / miss / closest-hit / any-hit, SBT, recursion).
  - Custom AABB intersection shaders, micromaps, partitioned TLAS.
  - Any web / WASM deployment of RT (WebGPU spec not there).
- **Migration trigger to `ash`**: when the engine needs ray-tracing pipelines — specifically, recursive rays driven by per-material closest-hit shaders or non-triangle custom-intersection primitives — AND wgpu's #8560 (Metal pipelines) is still unresolved. Until then, wgpu 29 ray query covers every realistic use case in the next 12 months, and switching earlier trades a working ecosystem for 5 % more hardware surface.

---

## F. Advanced rasterization

### F.1 Mesh shaders
- **Status**: experimental (shipped in wgpu 28, still gated by `EXPERIMENTAL_` flag in 29)
- **Notes**: `Features::EXPERIMENTAL_MESH_SHADER` (bit 48). Replaces the vertex pipeline; ideal for meshlet rendering. Backends: **Vulkan** (`VK_EXT_mesh_shader`) full WGSL via naga. **DX12** and **Metal** only with passthrough shaders (manual HLSL/MSL) — naga SPV-out / HLSL-out for mesh shaders is Vulkan-only. New API: `RenderPass::draw_mesh_tasks`, `draw_mesh_tasks_indirect`, `multi_draw_mesh_tasks_indirect`, and `*_count` variant. `EXPERIMENTAL_MESH_SHADER_POINTS` (bit 55, Vulkan + Metal) for point primitives; `EXPERIMENTAL_MESH_SHADER_MULTIVIEW` (bit 50, Vulkan-only in v29). Caveat: recommended to use `create_shader_module_trusted` with `ShaderRuntimeChecks::unchecked()` to avoid zero-init of workgroup memory single-threaded (expensive). Mesa LLVMPIPE fails; RADV OK.
- **Source**: [features.rs:1239-1293 @ wgpu-v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu-types/src/features.rs#L1239-L1293), [CHANGELOG v28 "Mesh Shaders"](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/CHANGELOG.md#mesh-shaders), [Tracking #7197](https://github.com/gfx-rs/wgpu/issues/7197), PRs [#7089](https://github.com/gfx-rs/wgpu/pull/7089) / [#8110](https://github.com/gfx-rs/wgpu/pull/8110) / [#8139](https://github.com/gfx-rs/wgpu/pull/8139) / [#7345](https://github.com/gfx-rs/wgpu/pull/7345).

### F.2 Task / amplification shaders
- **Status**: experimental (covered by the same flag as mesh shaders)
- **Notes**: The same `EXPERIMENTAL_MESH_SHADER` enables task shaders (`@task` in WGSL, emitting a `@builtin(mesh_task_size)` dispatch grid with `taskPayload`). Same backend mapping as F.1. No separate feature flag or entry point — both are stages of the same mesh-shader pipeline.
- **Source**: [CHANGELOG v28 — example shows `@task` + `@mesh`](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/CHANGELOG.md#mesh-shaders), [docs/api-specs/mesh_shading.md @ v29](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/docs/api-specs/mesh_shading.md).

### F.3 Variable Rate Shading
- **Status**: blocked (does not exist in wgpu 29)
- **Notes**: Exhaustive grep of `wgpu-types/src/features.rs` @ v29.0.1 finds no `*SHADING_RATE*`, `*VARIABLE_RATE*`, or `*VRS*`. CHANGELOG never mentions VRS. No active tracking issue in gfx-rs/wgpu (search only returns the unrelated "Sample shading #1122"). WebGPU spec has no merged proposal either (native-only feature). Workaround: render to a smaller target + upscale (FSR / TAAU) or multi-pass with scissor — not real VRS.
- **Source**: [features.rs full file @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu-types/src/features.rs) (absence), [issue search — "variable rate shading"](https://github.com/gfx-rs/wgpu/issues?q=variable+rate+shading).

### F.4 Bindless / descriptor indexing
- **Status**: supported (granular, production-ready on Vulkan / DX12; Metal much improved in v28)
- **Notes**: WGSL `binding_array<T, N>` available behind separate features:
  - `TEXTURE_BINDING_ARRAY` (bit 8) — DX12, Metal (MSL 2.0+), Vulkan.
  - `BUFFER_BINDING_ARRAY` (bit 9) — **Vulkan only** (DX12 / Metal not natively supported yet).
  - `STORAGE_RESOURCE_BINDING_ARRAY` (bit 10) — Metal (MSL 2.2+), Vulkan.
  - `UNIFORM_BUFFER_BINDING_ARRAYS` (bit 47) — DX12, Metal, Vulkan 1.2+ / `VK_EXT_descriptor_indexing`.
  - `SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING` (bit 11), `STORAGE_TEXTURE_ARRAY_NON_UNIFORM_INDEXING` (bit 12) — DX12, Metal 2.0+, Vulkan 1.2+.
  - `PARTIALLY_BOUND_BINDING_ARRAY` (bit 13) — Vulkan + DX12 Resource Binding Tier 3 (PR #6734).

  **Array size N** governed by two new limits (v28, PR #6811):
  - `max_binding_array_elements_per_shader_stage` — default 0 (unsupported) / **500 000 when bindless is supported** (up to 1 M on Intel legacy); same default in `downlevel_defaults`.
  - `max_binding_array_sampler_elements_per_shader_stage` — default 0 / 1 000 when bindless is supported.

  **Breaking rule (v28)**: if a bind group contains a `binding_array`, you cannot use dynamic-offset buffers or uniform buffers in the same bind group (requirement of Vulkan `UpdateAfterBind`). Our current pipeline passes the model matrix via dynamic offset → incompatible with co-locating a texture array there; future bindless work must segregate bind groups.
- **Source**: [features.rs:740-910 @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu-types/src/features.rs#L740-L910), [limits.rs:158-170 @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu-types/src/limits.rs#L158-L170), [CHANGELOG v28 — "Bindless support improved"](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/CHANGELOG.md), [Bindless Tracking #3637](https://github.com/gfx-rs/wgpu/issues/3637) (open).

### F.5 Multi-draw indirect
- **Status**: supported (breaking change in v28)
- **Notes**: **`Features::MULTI_DRAW_INDIRECT` was removed in v28** (PR [#8162](https://github.com/gfx-rs/wgpu/pull/8162)). `RenderPass::multi_draw_indirect` and `multi_draw_indexed_indirect` are now unconditional provided the adapter exposes `DownlevelFlags::INDIRECT_EXECUTION` (true on all modern backends — Vulkan / DX12 / Metal; the Deck's GL ES backend does not qualify). `_count` variants remain gated by `Features::MULTI_DRAW_INDIRECT_COUNT` (bit 15, DX12 + Vulkan 1.2 / `VK_KHR_draw_indirect_count`; Metal and OpenGL lack it). `INDIRECT_FIRST_INSTANCE` (bit 8 in WebGPU) allows `first_instance != 0` in the indirect buffer on Vulkan / DX12 / Metal. New related API: `multi_draw_mesh_tasks_indirect[_count]`. Both RDNA 4 and Steam Deck RDNA 2 support `_count` under Vulkan 1.2.
- **Source**: [CHANGELOG v28 — "Multi-draw indirect unconditionally supported" PR #8162](https://github.com/gfx-rs/wgpu/pull/8162), [features.rs:863-880 @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu-types/src/features.rs#L863-L880), [render_pass.rs:328-370 @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu/src/api/render_pass.rs#L328-L370).

### Area F summary
- **Feasible today**:
  - Multi-draw indirect without count on all targets (no feature flag, only `INDIRECT_EXECUTION`).
  - `MULTI_DRAW_INDIRECT_COUNT` on RDNA 4 / RDNA 2 under Vulkan and DX12.
  - Bindless texture arrays up to 500 k elements with `TEXTURE_BINDING_ARRAY` + `SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING` + `PARTIALLY_BOUND_BINDING_ARRAY` on Vulkan / DX12.
  - `INDIRECT_FIRST_INSTANCE` cross-backend for instance-ID offsets in indirect draws.
- **Requires workaround**:
  - Mesh / task shaders outside Vulkan: WGSL does not cross-compile to HLSL / MSL for this stage → requires `PASSTHROUGH_SHADERS` with manual HLSL / MSL on DX12 / Metal. Additionally, the bind group currently using dynamic offset (our model uniform) must be restructured before mixing with `binding_array`.
  - `BUFFER_BINDING_ARRAY` (heterogeneous uniform-buffer arrays) is Vulkan-only today — on DX12 / Metal, degrade to a single large storage buffer indexed manually.
- **Blocked**:
  - VRS (per-draw, per-primitive, or image-based): not in wgpu 29 and no tracking issue. Drop from roadmap until upstream proposes it.
  - Mesh shaders with multiview on DX12 / Metal (Vulkan-only in v29).
- **Project-specific recommendation (large-world streaming horizon)**: target **bindless + multi-draw indirect** (without count for Deck compatibility, with `_count` as a fast path on RDNA 4) as the base of the large-world renderer — the only production-ready cross-backend path. Keep **mesh shaders in an isolated engine feature flag**, valid only when the adapter reports Vulkan + `EXPERIMENTAL_MESH_SHADER` (RX 9070 XT yes, Deck likely no under RADV today); do not couple them to the deferred pipeline until naga ships stable SPV-out for DX12 / Metal.

---

## G. Ergonomics / debugging

> **Status: research pending.** Tracked as follow-up; see end of doc.

Planned investigation: timestamp queries per-pass / per-draw, pipeline statistics, RenderDoc label propagation via wgpu-hal, naga-compatible shader debugging tooling.

---

## H. Deployment / packaging

> **Status: research pending.** Tracked as follow-up; see end of doc.

Planned investigation: cross-backend naga coverage for our shaders, `wgpu 29 PipelineCache` status (we currently pass `cache: None`), runtime size of the wgpu crate on Windows / Linux / Steam Deck.

---

## Blocked features (partial — Areas A, E, F)

Ranked by roadmap pain.

| Feature | Area | Why blocked | Roadmap impact |
|---|---|---|---|
| Ray-tracing pipelines (raygen / miss / closest-hit, SBT, recursion) | E | Not in wgpu 29; tracker #6762 + #8560 unresolved. | **Medium** — ray query covers real-time GI, shadow, AO, and primary-visibility BVH replacement. Pipelines only blocked if we need per-material closest-hit shaders or custom-intersection primitives. |
| Custom AABB intersection shaders / micromaps / partitioned TLAS | E | Pending on tracker #6762. | Low today; relevant only when we add procedural-primitive RT (SDF-in-BVH hybrid). |
| Variable Rate Shading (per-draw / per-primitive / image-based) | F | Absent in wgpu 29; no tracking issue; no WebGPU proposal. | Low — we are not perf-bound yet. Keep off roadmap indefinitely. |
| Single-pass layered rendering (`gl_Layer`-style arbitrary layer output) | A | WebGPU has no geometry shaders; MULTIVIEW is view-index only. | Low — 6-pass cubemap render is fine for sky pre-bake cadence. |
| `Rgb9e5Ufloat` as render target or storage | A | Spec-locked read-only. | Low — use `Rgba16Float` as render target; `Rgb9e5Ufloat` is ideal as load-only HDRI source. |
| Mesh-shader cross-compile to HLSL / MSL | F | naga emits mesh shaders on Vulkan only in v29. | Low — only affects mesh-shader path on DX12 / Metal; passthrough HLSL / MSL is an acceptable escape hatch until naga catches up. |
| Web / WASM deployment of ray tracing | E | WebGPU RT proposal gpuweb#535 is Milestone 4+. | **Zero** — engine targets native Linux + Windows + Steam Deck. Not a loss. |

---

## Migration triggers (partial — Areas A, E, F)

Concrete signals that would justify dropping wgpu for `ash` or a hybrid. **None of these are active today.**

1. **RT-pipeline requirement**: engine needs recursive rays driven by per-material closest-hit shaders **and** wgpu #8560 is still unresolved after 12 months. → migrate the RT pass only to `ash` (hybrid), keep rasterization on wgpu. Bevy Solari's decision to stay on wgpu ray query is a strong "don't migrate prematurely" precedent.
2. **Steam Deck renders Vulkan 1.2 mesh-shader path uniformly**: if RADV gets full `VK_EXT_mesh_shader` coverage on RDNA 2 and we need the meshlet pipeline for large-world rendering → no migration, just flip the `EXPERIMENTAL_MESH_SHADER` engine flag on the Deck target.
3. **wgpu removes `EXPERIMENTAL_RAY_QUERY` without stabilizing**: unlikely, but would force `ash` adoption for any GI work. Mitigation: pin `wgpu` version at time of adoption, do not auto-bump.
4. **Non-wgpu feature becomes roadmap-critical** (VRS specifically): would require `ash`. Currently nothing in this engine needs VRS; re-evaluate if foveated rendering becomes a deliverable.

**Non-triggers (do NOT migrate for these)**:
- Missing high-level helpers (mipmap generation, IBL prefiltering) — writing compute shaders is the normal cost.
- Single-pass layered rendering — 6-pass cubemap is fine.
- Web target RT — we do not ship web.

---

## Recommendation — 12 / 24 months (partial)

**12-month horizon (≈ April 2027)**: `wgpu 29` (and its successors) cover the full rendering roadmap through G-Buffer deferred, sky / environment, mesh loading + texturing + PBR, and real-time GI via ray query. No migration trigger will fire. Stay on wgpu, pin version per release-please cycle, bump only on explicit changelog review. Keep `EXPERIMENTAL_*` features gated behind engine feature flags (`Engine::uses_mesh_shaders()` etc.) with a graceful fallback path.

**24-month horizon (≈ April 2028)**: the only credible pressure comes from RT pipelines. If by early 2028 ray-tracing pipelines are still absent (#8560 unresolved) and the engine has concrete demand for per-material closest-hit shaders — schedule a **targeted hybrid** (wgpu rasterization + ash RT pass) with a ~4-week spike. Do not rewrite the engine on `ash`; do not port the web backend to anything. VRS and other perf knobs remain optional and do not justify migration.

**Investigation remaining (Areas B/C/D/G/H)**: follow-up issues below. None of them are expected to produce a migration trigger — they should confirm or add nuance to the 12-month "stay on wgpu" recommendation.

---

## Follow-up research issues to open

- `research(render): wgpu deferred pipeline capabilities (area B)` — MRT attachment limits, depth-as-texture in second pass, storage texture atomics.
- `research(render): wgpu compute capabilities (area C)` — workgroup size on RADV / RDNA 4, subgroup ops, compute↔fragment interop, indirect dispatch.
- `research(render): wgpu volumetric / 3D texture capabilities (area D)` — 3D storage, 3D sampled, volume arrays.
- `research(render): wgpu ergonomics / debugging (area G)` — timestamp + pipeline statistics queries, RenderDoc labels, shader debugging.
- `research(render): wgpu deployment / packaging (area H)` — naga cross-backend coverage, `PipelineCache` usage, runtime footprint.

Each of these should follow the same format as Areas A / E / F (status + sourced notes per sub-question, area summary with "feasible today / workaround / blocked", project-specific recommendation). They are expected to roll up into the **Blocked features**, **Migration triggers**, and **Recommendation** sections of this doc.
