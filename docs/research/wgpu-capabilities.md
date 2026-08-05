# wgpu Capabilities & Limitations Audit

**Scope**: authoritative, project-grounded audit of what `wgpu 29` (our current pinned version) can and cannot do, so future rendering features can be designed against a known capability matrix instead of ad-hoc research per issue.

**Related issue**: [#238](https://github.com/lobinuxsoft/kooch/issues/238).
**Pinned version**: `wgpu = "29"` (workspace `Cargo.toml`).
**Target hardware**: AMD RDNA 4 (RX 9070 XT) on Linux Bazzite / RADV primary; Windows and Steam Deck RDNA 2 as secondary targets.
**Status as of this revision**: **complete — Areas A through I cover the full audit.**

## Table of contents

- [Status legend](#status-legend)
- [A. Sky / environment](#a-sky--environment)
- [B. Deferred pipeline / multi-pass](#b-deferred-pipeline--multi-pass)
- [C. Compute shaders](#c-compute-shaders)
- [D. Volumetric / 3D textures](#d-volumetric--3d-textures)
- [E. Hardware-accelerated ray tracing](#e-hardware-accelerated-ray-tracing)
- [F. Advanced rasterization](#f-advanced-rasterization)
- [G. Ergonomics / debugging](#g-ergonomics--debugging)
- [H. Deployment / packaging](#h-deployment--packaging)
- [I. Post-processing effects](#i-post-processing-effects)
- [Blocked features (all areas)](#blocked-features-all-areas)
- [Migration triggers](#migration-triggers)
- [Recommendation — 12 / 24 months](#recommendation--12--24-months)

## Status legend

| Tag | Meaning |
|---|---|
| **supported** | Works in `wgpu 29` out of the box on our target backends (Vulkan + DX12 primary, Metal secondary). No feature flag required, or the required flag is stable. |
| **experimental** | Exposed behind an `EXPERIMENTAL_*` feature flag; API may shift; known bugs tracked upstream. Usable with caution. |
| **workaround** | The desired capability is not a first-class API but can be built from lower-level primitives already shipped. |
| **blocked** | Not available in `wgpu 29` and no accepted proposal in sight. Requires either waiting upstream or leaving `wgpu`. |

All claims are sourced. When a claim could not be verified against a primary source (wgpu release notes, gfx-rs tracker, WebGPU/WGSL spec, naga docs), it is marked `unverified`.

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
  - `Rg11b10Ufloat` as render target: gated by `Features::RG11B10UFLOAT_RENDERABLE`; check at runtime.
  - Mipmap / IBL prefiltering: no official helper; implement ad-hoc compute shaders using the `mipmap` example as reference.
  - `Rgba32Float` filterable / blendable: feature-gated.
- **Blocked**:
  - Arbitrary `gl_Layer`-style layered rendering in a single draw. No geometry shaders in WebGPU.
  - `Rgb9e5Ufloat` as render target or storage — spec-locked read-only.
- **Project-specific SkyRenderer recommendation**: use `Rgba16Float` as canonical format for both runtime cubemap and its prefiltered mip chain (spec-guaranteed MSAA-resolve + storage + filtering), load HDRIs as `Rgb9e5Ufloat` equirect (read-only sampling), and convert to cube via a **single compute pass** emitting into `texture_storage_2d_array<rgba16float, write>` with 6 layers. Avoids 6 render passes, depends on no optional feature, stays compatible with RDNA 2 (Steam Deck) as well as RDNA 4.

---

## B. Deferred pipeline / multi-pass

### B.1 Multiple render targets (MRT)
- **Status**: supported. Default `Limits::max_color_attachments = 8` on native; downlevel (WebGL2 / Metal tier 1) caps at `4`. Per-pass total bandwidth gated by `max_color_attachment_bytes_per_sample = 32` default.
- **Notes**: Mixed formats per pass (e.g. `Rgba16Float` + `Rgba8Unorm`) allowed if summed per-format pixel byte cost fits the bytes-per-sample budget; WebGPU also requires all color targets in a pass to share the same sample count. A naive G-Buffer of 4 × `Rgba16Float` = 32 B already saturates the default budget — either request a higher limit or downsize channels (pack normals into `Rg16Snorm`, material into `Rgba8Unorm`). RDNA 4 Vulkan reports `maxColorAttachments ≥ 8` and `maxColorAttachmentBytesPerSample ≥ 64` comfortably. Steam Deck RDNA 2 idem on Vulkan; design to the downlevel `4`/`32B` only if a WebGL fallback is ever added.
- **Source**: [limits.rs @ v29.0.1 L205–211, L340–402, L461–508, L542–545](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/limits.rs), [WebGPU spec — render pass encoding](https://www.w3.org/TR/webgpu/#render-pass-encoder-creation).

### B.2 Depth as texture in later pass
- **Status**: supported.
- **Notes**: Create depth with `TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING`, format `Depth32Float`, bind in the next pass as `texture_depth_2d` (filtering requires a `depth_compare` sampler; otherwise use `texture_depth_multisampled_2d` or a non-filtering sampler). wgpu-hal inserts the Vulkan `VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_OPTIMAL` transition automatically between passes in the same `CommandEncoder`. Two gotchas: (1) while the depth texture is sampled, set `depth_ops: None` or `depth_read_only: true` — otherwise validation rejects the second pass; (2) MSAA depth cannot be sampled directly as `texture_depth_2d` — resolve or copy first.
- **Source**: [WebGPU spec — Depth/stencil attachment](https://www.w3.org/TR/webgpu/#depth-stencil-attachment), [WGSL spec — Depth textures](https://www.w3.org/TR/WGSL/#texture-depth).

### B.3 Storage textures + atomics
- **Status**: supported natively in v29 for `R32*` formats.
- **Notes**: `Features::TEXTURE_ATOMIC` (bit 28) enables image atomics (`add, and, or, xor, min, max, exchange`) on `R32Uint`/`R32Sint`. `Features::TEXTURE_INT64_ATOMIC` (bit 46) enables 64-bit `min/max` on `R64Uint`. Read-write storage textures for richer formats (`Rgba16Float` OIT accumulators) still gated on upstream WebGPU `RW_STORAGE_TEXTURE_TIER_1` discussion — explicit `// ? const RW_STORAGE_TEXTURE_TIER_1` marker in features.rs L629. Today storage textures default to write-only; read/write only via `access: read_write` on `R32*` / `Rg32*` / `Rgba32*`. No atomics on `F32` storage textures.
- **Source**: [features.rs @ v29.0.1 L612–629, L990–999, L1210–1219](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/features.rs), [v29.0.0 release notes — "Work around Metal driver bug with atomic textures"](https://github.com/gfx-rs/wgpu/releases/tag/v29.0.0), [gpuweb#3838](https://github.com/gpuweb/gpuweb/issues/3838) (`unverified` for wgpu timeline).

### B.4 Load/store ops + tile-based cost
- **Status**: supported per attachment.
- **Notes**: `LoadOp::{Clear(v), Load}`, `StoreOp::{Store, Discard}` (renamed from the old `bool`); same shape for depth and stencil via `RenderPassDepthStencilAttachment::{depth_ops, stencil_ops}`. Steam Deck Van Gogh RDNA 2 is immediate-mode (not true TBDR like Mali/Apple), so the tile-cache benefit is smaller than on mobile — but DCC + tile cache still helps. Use `LoadOp::Clear` whenever the attachment is fully overwritten (avoids framebuffer fetch), `StoreOp::Discard` on attachments never sampled after the pass (stencil aux, intermediate MSAA targets being resolved). G-Buffer MRTs must be `StoreOp::Store` — the lighting pass samples them.
- **Source**: [WebGPU spec — GPUStoreOp / GPULoadOp](https://www.w3.org/TR/webgpu/#enumdef-gpustoreop).

### B.5 Pass chaining (no subpasses)
- **Status**: workaround (wgpu 29 does not expose Vulkan subpasses or DX12 render-pass input attachments).
- **Notes**: Idiomatic chain is separate `RenderPass`es in the same `CommandEncoder`, one writing, the next sampling via `TEXTURE_BINDING`. wgpu-hal inserts the minimal barrier automatically. `TextureUsages::STORAGE_BINDING` enables a compute or later fragment pass with `access: write` to write to the texture — useful for light-culling tiles / SSAO downsample — but forbids using the texture as color attachment in the same pass. No "input attachment" same-pixel coherent read path exists; classic deferred where the lighting shader reads the very same tile being produced must accept the off-chip round-trip. Mitigation: group passes tightly in one encoder so drivers can merge render passes (Vulkan) or keep resources in L2 (DX12); avoid `queue.submit` between G-Buffer fill and lighting.
- **Source**: [v29.0.0 release notes](https://github.com/gfx-rs/wgpu/releases/tag/v29.0.0) (no subpass entry — consistent with absence of support), [WebGPU spec — render pass encoder](https://www.w3.org/TR/webgpu/#render-pass-encoder) (no subpass concept by design).

### Area B summary
- **Feasible today**: 4-target G-Buffer with mixed `Rgba16F` / `Rgba8Unorm` / `Depth32F` attachments; sampling depth-as-texture in subsequent passes; per-attachment Load/Store/Discard; storage-texture writes + `R32*` image atomics via `Features::TEXTURE_ATOMIC`.
- **Requires workaround**: per-pixel linked-list OIT on rich formats — `R32Uint` head-pointer texture + storage buffer for payload (read/write `Rgba16Float` storage textures not yet standardized); same-pixel input-attachment reads — separate passes and accept the L2 round-trip.
- **Blocked**: true Vulkan subpass merging / framebuffer-fetch; WebGPU-spec storage-texture `Rgba16F` atomics.
- **Project-specific G-Buffer recommendation**: pack into 4 MRTs fitting ≤ 32 B/sample — albedo `Rgba8Unorm` + packed normal `Rg16Snorm` + material `Rgba8Unorm` + motion `Rg16Float` — with `Depth32Float` as a 5th read-only attachment sampled by the lighting pass. Request `Features::TEXTURE_ATOMIC` only when OIT or per-pixel linked-lists land (not critical path for #132). Structure each frame as one `CommandEncoder` with GBuffer-fill → lighting → post stack back-to-back so wgpu-hal can fuse barriers.

---

## C. Compute shaders

### C.1 Workgroup size limits
- **Status**: supported.
- **Notes**: `wgpu::Limits` defaults (v29.0.1):

  | Limit | default | downlevel | WebGL2 |
  |---|---|---|---|
  | `max_compute_workgroup_size_x` | 256 | 256 | 0 |
  | `max_compute_workgroup_size_y` | 256 | 256 | 0 |
  | `max_compute_workgroup_size_z` | 64 | 64 | 0 |
  | `max_compute_invocations_per_workgroup` | 256 | 256 | 0 |
  | `max_compute_workgroup_storage_size` | 16384 | 16352 | 0 |
  | `max_compute_workgroups_per_dimension` | 65535 | 65535 | 0 |

  WebGL2 zeros out compute entirely (no compute shaders on WebGL2). Hardware ceilings are higher: RADV (Vulkan) on RDNA 2/3/4 typically reports `maxComputeWorkGroupInvocations = 1024` and `maxComputeSharedMemorySize` 32 KiB or 64 KiB. wgpu clamps to its default unless you explicitly request higher via `DeviceDescriptor::required_limits`. **Call `Adapter::limits()` at startup and request `max_compute_invocations_per_workgroup = 1024` + `max_compute_workgroup_storage_size = 32768`** when available — efficient bloom downsample and SSAO tile sizes depend on it.
- **Source**: [limits.rs @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/limits.rs), [Limits docs.rs](https://docs.rs/wgpu/29.0.1/wgpu/struct.Limits.html).

### C.2 Subgroup / wave intrinsics
- **Status**: supported.
- **Notes**: From features.rs v29.0.1:
  - `Features::SUBGROUP` — Vulkan + DX12 + Metal. Non-`wgpu-` prefix → expected to graduate into WebGPU core.
  - `Features::SUBGROUP_BARRIER` — Vulkan + Metal (**no DX12**).
  - `Features::SUBGROUP_VERTEX` — Vulkan only.

  WGSL spec builtins (§17.12): `subgroupBroadcast`, `subgroupBallot`, `subgroupAdd`, `subgroupMin`, `subgroupMax`, `subgroupShuffle`, `subgroupElect`, `subgroupAll`, `subgroupAny`. Builtin values `subgroup_size` and `subgroup_invocation_id`. `subgroupBarrier` is gated behind `SUBGROUP_BARRIER` as wgpu extension. Quad ops landed in v28 under `SUBGROUP`.

  Subgroup size queryable at runtime via `AdapterInfo::subgroup_min_size` / `subgroup_max_size`. Hardware: Nvidia 32, Intel 8/16/32, AMD RDNA 2/3/4 use Wave32 on consumer GPUs (RX 9070 XT = Wave32) but drivers may report min=32/max=64. **Never hardcode** — read from `AdapterInfo` or use the `subgroup_size` WGSL builtin.
- **Source**: [features.rs @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/features.rs), [WGSL spec §17.12](https://www.w3.org/TR/WGSL/#subgroup-builtin-functions), [AdapterInfo docs.rs](https://docs.rs/wgpu/29.0.1/wgpu/struct.AdapterInfo.html), [CHANGELOG v28 (quad ops)](https://github.com/gfx-rs/wgpu/blob/v29.0.1/CHANGELOG.md).

### C.3 Compute → fragment interop
- **Status**: supported (via implicit pass-boundary sync).
- **Notes**: WebGPU defines each pass as a separate "usage scope" and exposes no manual barrier API; sync between a compute pass writing a storage texture and a later render pass sampling it is guaranteed by queue submission order + wgpu's internal tracking. Within a single compute pass the only primitives are WGSL `workgroupBarrier`, `storageBarrier`, `textureBarrier`. `wgpu::CommandEncoder` in v29 has no `memory_barrier`/`pipeline_barrier` methods. The only escape hatch is the native-only `CommandEncoder::transition_resources` (for batching hal transitions across command buffers, not general sync). You get correctness for free but lose the ability to overlap independent passes without multi-queue submission (wgpu does not expose multi-queue in v29).
- **Source**: [WebGPU spec — synchronization](https://www.w3.org/TR/webgpu/#programming-model-synchronization), [CommandEncoder docs.rs](https://docs.rs/wgpu/29.0.1/wgpu/struct.CommandEncoder.html).

### C.4 Indirect dispatch
- **Status**: supported.
- **Notes**: `ComputePass::dispatch_workgroups_indirect` has **no feature gate** — it's core API. Availability is via downlevel capability `DownlevelFlags::INDIRECT_EXECUTION` (which requires `COMPUTE_SHADERS`). **Confirmed unsupported** on WebGL2 / GLES 3.0 / Metal on Apple1/Apple2. Steam Deck RADV has both flags — indirect dispatch works. `MULTI_DRAW_INDIRECT_COUNT` (native-only, Vulkan 1.2+ / DX12) covers the count-buffer variant for GPU-driven culling.
- **Source**: [ComputePass docs.rs](https://docs.rs/wgpu/29.0.1/wgpu/struct.ComputePass.html), [DownlevelFlags docs.rs](https://docs.rs/wgpu/29.0.1/wgpu/struct.DownlevelFlags.html), [downlevel.rs @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/downlevel.rs).

### C.5 Workgroup (shared) memory
- **Status**: supported (default 16 384 B, can request more).
- **Notes**: WebGPU-mandated zero-init of `var<workgroup>` is implemented by wgpu; since v0.20 it's configurable per-pipeline. Opt out via `Device::create_shader_module_trusted(desc, ShaderRuntimeChecks::unchecked())`. Cost is non-trivial on backends without native LDS clear: naga emits a loop every invocation runs + `workgroupBarrier` before your shader body, measurable on large (16 KiB) allocations. **Recommendation**: keep zero-init on for post-FX shaders (negligible cost at 4–8 KiB LDS); flip to `unchecked()` for SSAO tiles / bloom pyramid where LDS is fully overwritten before read.
- **Source**: [wgpu #3492](https://github.com/gfx-rs/wgpu/issues/3492), [Bevy #16301](https://github.com/bevyengine/bevy/pull/16301).

### C.6 Push constants (renamed → "immediates")
- **Status**: supported.
- **Notes**: **v28 breaking change**: `PUSH_CONSTANTS` → `Features::IMMEDIATES`; `Limits::max_push_constant_size` → `max_immediate_size`; WGSL `var<push_constant>` → `var<immediate>`; `set_push_constants()` → `set_immediates()`. This is the v29 name.
  - `Features::IMMEDIATES` — Vulkan + DX12 + Metal + OpenGL (emulated via uniforms) + WebGPU.
  - `Limits::max_immediate_size` default 0 (must request). Backend caps: Vulkan 128–256 B, DX12 128 B, Metal 4096 B, GL ~256 B. **Conservative budget: 128 B** — safe everywhere. No Metal argument-buffer workaround needed.
- **Source**: [CHANGELOG v28 — "Push constants renamed immediates"](https://github.com/gfx-rs/wgpu/blob/v29.0.1/CHANGELOG.md), [features.rs @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/features.rs).

### Area C summary
- **Feasible today**: core compute dispatch, indirect dispatch, subgroup ops on Vulkan / DX12 / Metal (including RDNA 4 + Deck via RADV), immediates ≤128 B, WGSL `workgroupBarrier` / `storageBarrier`, automatic compute→render sync, adapter-reported subgroup size probing.
- **Requires workaround**: LDS zero-init cost on large allocations → opt into `ShaderRuntimeChecks::unchecked()` only for shaders that self-initialize; request elevated `required_limits` for 1024-invocation workgroups + 32 KiB LDS; `SUBGROUP_BARRIER` absent on DX12 → any cross-lane sync beyond implicit subgroup-op semantics must branch Vulkan-only or fall back.
- **Blocked**: manual GPU-side barriers (no API surface); `SUBGROUP_VERTEX` on DX12 / Metal; multi-queue async compute (single queue in v29); GL ES / WebGL fallback for compute.
- **Project-specific recommendation**: use compute for post-processing and GPU-driven culling unconditionally — RDNA 4 and Deck RDNA 2 hit every feature we need; pass-boundary auto-sync keeps the hot loop (compute bloom → fragment composite) trivially correct. Keep `IMMEDIATES` at 128 B for cross-backend parity. Treat future physics as **Vulkan-preferred** so `SUBGROUP_BARRIER` + full RADV wave32 toolchain are available; DX12 physics is stretch goal, not requirement.

---

## D. Volumetric / 3D textures

### D.1 3D sampled textures
- **Status**: supported.
- **Notes**: `TextureDimension::D3` and `TextureViewDimension::D3` exist in v29 (`wgpu-types/src/texture.rs:35,90`). WGSL exposes `texture_3d<T>` with sampling via `textureSample` / `textureSampleLevel`. Trilinear filtering via `Sampler` with `address_mode_w`, `mag_filter`, `min_filter`, `mipmap_filter` all `Linear`. WebGPU guarantees linear 3-axis filtering for formats marked `FILTERABLE`: `Rgba8Unorm`, `Rgba16Float`, `Rgb9e5Ufloat` all qualify by spec. `Rgba32Float` requires `Features::FLOAT32_FILTERABLE`. `Limits::max_texture_dimension_3d` default 2048 (downlevel 256). RDNA 4 reports 16 384 via Vulkan `maxImageDimension3D`, but wgpu clamps to default unless you request explicitly.
- **Source**: [texture.rs @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/texture.rs), [format.rs @ v29.0.1 L1086–L1148](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/texture/format.rs), [WGSL §8.9 Texel Formats and Sampling](https://www.w3.org/TR/WGSL/).

### D.2 3D storage textures
- **Status**: supported (with format-specific read-write restrictions).
- **Notes**: WGSL `texture_storage_3d<format, access>` supported; binding requires `TextureUsages::STORAGE_BINDING`. Access modes: `ReadOnly`, `WriteOnly`, `ReadWrite`, `Atomic`. `STORAGE_READ_WRITE` guaranteed only on `R32Uint`, `R32Sint`, `R32Float`. The rest (`Rgba16Float`, `Rgba8Unorm`, etc.) is write-only by spec; read-write requires `Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` (non-portable). `Rgb9e5Ufloat` is **not a storage format** (only binding + copy). `Bgra8Unorm` storage requires `BGRA8UNORM_STORAGE`.
- **Source**: [format.rs @ v29.0.1 L917–990, L965–967, L1011, L1944–1953](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/texture/format.rs), [features.rs @ v29.0.1 L678, L1686](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/features.rs).

### D.3 3D texture arrays
- **Status**: blocked.
- **Notes**: WebGPU / wgpu **do not expose `texture_3d_array`**. `TextureViewDimension` enum only has `D1, D2, D2Array, Cube, CubeArray, D3`. naga validation maps `ImageDimension::D3` exclusively to `D3`. Workaround: pack N volumes into a single `texture_3d` extended along Z (`Z × N`) with manual offsetting in-shader, or use `texture_2d_array` for 2D slices if strict 3D access isn't required. For small irradiance volumes (8³), many probes fit comfortably in a 3D atlas.
- **Source**: [texture.rs @ v29.0.1 L71–91](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/texture.rs), [wgpu-core validation.rs @ v29.0.1 L539, L679](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-core/src/validation.rs).

### D.4 Mipmapping of 3D textures
- **Status**: workaround.
- **Notes**: `Extent3d::max_mips(TextureDimension::D3)` considers all 3 dimensions. A 256³ volume gives `log2(256)+1 = 9` mip levels. **No built-in generator**. Strategies:
  - Compute per level: bind level N as `texture_3d` sampled, level N+1 as `texture_storage_3d<rgba16float, write>`, dispatch `ceil(dim/8)` groups of 8×8×8. Works with write-only formats.
  - Render pass per Z-slice: one pass per slice of level N+1 via `TextureViewDescriptor { dimension: D3, base_array_layer: z, array_layer_count: 1, .. }` — slower but works for non-storage formats.

  Minification: sampler with all-`Linear` filters → trilinear between mip levels (quadrilinear over 3 axes + LOD).
- **Source**: [texture.rs @ v29.0.1 L1083–L1092](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/texture.rs), [WebGPU spec §23 Sampling](https://www.w3.org/TR/webgpu/#sampling).

### D.5 Memory / bandwidth cost (256³ = 16 777 216 texels)

| Format         | Bytes/texel | Mip 0        | + full mipchain (~1.143×) |
|----------------|-------------|-------------:|--------------------------:|
| `Rgba16Float`  | 8           | **128 MiB**  | ~146 MiB                  |
| `Rgb9e5Ufloat` | 4           | **64 MiB**   | ~73 MiB                   |
| `Rgba8Unorm`   | 4           | **64 MiB**   | ~73 MiB                   |

Upload via `Queue::write_texture` is bandwidth-limited by PCIe 4.0 x16 (~28 GiB/s theoretical on RDNA 4). `Rgba16Float` 256³ = 128 MiB ≈ 4.6 ms theoretical, realistic 10–20 ms (`unverified` — measure with `Queue::on_submitted_work_done`). `Rgb9e5Ufloat` gives **2× density vs `Rgba16Float`** with shared HDR exponent — ideal for fog / clouds where alpha is not needed. Trade-off: not a storage format, must be generated offline or via render pass.

### D.6 3D LUT for color grading
- **Status**: supported.
- **Notes**: Industry standard: **32³** (64 KiB in `Rgba16Float`, 32 KiB in `Rgba8Unorm`) — OpenColorIO, Resolve, Unreal. **64³** (512 / 256 KiB) for high-fidelity HDR filmic. Format recommendations:
  - LDR / sRGB input → `Rgba8Unorm` (filterable guaranteed).
  - HDR linear input → `Rgba16Float` (filterable guaranteed).
  - `Rgba32Float` works but needs `FLOAT32_FILTERABLE` for trilinear (unnecessary for LUTs).

  Trilinear is spec-guaranteed for both recommended formats because they carry `FILTERABLE`. Sampler: `mag_filter=Linear, min_filter=Linear, mipmap_filter=Nearest` (LUT has no mips), address modes `ClampToEdge` in u/v/w — critical to avoid wrap artifacts at cube edges.

### Area D summary
- **Feasible today**: 3D sampled textures in `Rgba16Float` / `Rgb9e5Ufloat` / `Rgba8Unorm` with trilinear; 3D storage read/write on `R32Uint/Sint/Float`; 3D storage read-only or write-only on Rgba8/16/32 non-sRGB; 3D LUT 32³ / 64³ with guaranteed filtering.
- **Requires workaround**: arrays of 3D textures → tile into a larger 3D texture (Z extension) or use `texture_2d_array`; mip generation → compute preferred; storage read-write on non-`R32*` → requires `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`, not portable.
- **Blocked**: `Rgb9e5Ufloat` as storage target (spec: no storage, no render attachment) — must write to `Rgba16Float` and convert offline, or derive analytically in-shader for procedural fog.
- **Project-specific recommendation**: for **volumetric fog / clouds** use `texture_storage_3d<rgba16float, write>` 160×90×128 written by compute (Frostbite-style froxel grid), sampled with trilinear + temporal jitter. For **3D color grading LUT** use `Rgba16Float` 32³ if the pipeline is HDR (input post-tonemap linear) or `Rgba8Unorm` 32³ if SDR — both guarantee trilinear without extra features and fit comfortably in L2.

---

## E. Hardware-accelerated ray tracing

### E.1 Current wgpu 29 state
- **Status**: experimental — ray queries (inline RT) shipping; ray-tracing pipelines still in development.
- **Notes**: wgpu 29 exposes `Features::EXPERIMENTAL_RAY_QUERY`. Unlocks `Device::create_blas` / `create_tlas`, BLAS compaction (`prepare_compaction_async` → `Queue::compact_blas`), 24-bit custom instance data in TLAS, and ray-query intrinsics in WGSL (`rayQueryInitialize`, `rayQueryProceed`, `rayQueryGetCommittedIntersection`, `getCommittedHitVertexPositions`, etc.). WGSL must declare `enable wgpu_ray_query;` (+ `wgpu_ray_query_vertex_return;` for vertex fetch). The spec doc explicitly warns "may contain major bugs". SBT record offset returns `0` (reserved). `@any_hit` shaders cannot call `traceRay()`. Ray-tracing **pipelines** (miss / closest-hit / any-hit shader tables) are NOT in the public v29 API surface.
- **Source**: [docs/api-specs/ray_tracing.md @ v29](https://github.com/gfx-rs/wgpu/blob/v29/docs/api-specs/ray_tracing.md), [Features @ docs.rs](https://docs.rs/wgpu/latest/wgpu/struct.Features.html).

### E.2 Tracker state
- Authoritative tracker: [gfx-rs/wgpu#6762 "Ray Tracing Tracking Issue"](https://github.com/gfx-rs/wgpu/issues/6762), opened 2024-12-16, **open** as of April 2026. Supersedes [#1040](https://github.com/gfx-rs/wgpu/issues/1040) (closed as not planned).
- Backend coverage for ray query + acceleration structures: Vulkan ✅, DX12 ✅ ([PR #6777](https://github.com/gfx-rs/wgpu/pull/6777)), Metal ✅ ([PR #8071](https://github.com/gfx-rs/wgpu/pull/8071)). ~12 known bugs: AMD GPU test failures ([#6727](https://github.com/gfx-rs/wgpu/issues/6727)), Mesa RADV segfaults, Vulkan alignment, Metal sync, UB in ray queries ([#6761](https://github.com/gfx-rs/wgpu/issues/6761)).
- Pending: custom AABB intersections, micromap support, partitioned TLASes, `instance_id` → `instance_index` rename, ray-tracing pipeline basic design. Open blocker: [#8560 "Should Metal have ray tracing pipelines?"](https://github.com/gfx-rs/wgpu/issues/8560) — unresolved design question gating cross-backend pipeline work. Earlier pipeline work ([PR #3607](https://github.com/gfx-rs/wgpu/pull/3607)) closed-unmerged after dependency drift.

### E.3 naga / WGSL
- naga parses and emits ray-query constructs end-to-end: SPIR-V, HLSL (35 ray-query intrinsics), MSL backends. Gated by `enable wgpu_ray_query` / `enable wgpu_ray_query_vertex_return` WGSL extension directives.
- naga does **not** yet have IR for ray-tracing-pipeline shader stages (raygen / miss / closest-hit / any-hit / intersection) nor SBT indexing. Gating item on #6762 for pipeline support.
- **Source**: [ray_tracing.md @ trunk](https://github.com/gfx-rs/wgpu/blob/trunk/docs/api-specs/ray_tracing.md).

### E.4 Realistic timeline
- **12 months (≈ April 2027)**: ray-query path stabilizes, `EXPERIMENTAL_` prefix likely dropped, bugs closed, custom AABB intersections possibly land. Ray-tracing pipelines probably still absent on Metal (#8560 still open). Not expected non-experimental on DX12 until parity bugs close.
- **24 months (≈ April 2028)**: ray-tracing pipelines plausible on Vulkan + DX12 if a maintainer picks up the Metal question. No milestone commitment. WebGPU-standard RT [gpuweb#535](https://github.com/gpuweb/gpuweb/issues/535) assigned to "Milestone 4+" (undated) — web trails native by years. Any 2028 delivery is contributor-driven, not roadmap-driven.

### E.5 Alternatives

#### ash (raw Vulkan)
- Mature, actively maintained, thin bindings. `VK_KHR_acceleration_structure` + `VK_KHR_ray_tracing_pipeline` + `VK_KHR_ray_query` all exposed. Cost: manual descriptor + sync; lose cross-platform (Linux + Windows only — acceptable; Metal and browser cut). Reasonable "RT pass only" path because wgpu-hal already depends on ash; `Device::as_hal::<vulkan::Api>` lets you unwrap.

#### vulkano
- Exposes `vulkano::pipeline::ray_tracing`. Pre-1.0 with regular breakage and prolonged contributor gaps. Adequate for hobby, risky for a long-lived engine. Not recommended while ash solves the same problem with a thinner, more stable surface.

#### Hybrid (wgpu + ash)
- Technically realistic: `wgpu_hal::vulkan::Device::texture_from_raw` + `Device::as_hal::<vulkan::Api>` share `VkImage` / timeline semaphores. Queue family + layout transitions are the gotcha — RT pass must emit barrier wgpu expects on re-entry.
- Public exemplars are thin. Bevy's [Solari @ 0.17](https://jms55.github.io/posts/2025-09-20-solari-bevy-0-17/) stayed on upstream wgpu ray query instead of hybrid — strong signal that ray query is "good enough" for real-time GI today and hybrid is not widely validated.
- Adopt hybrid only if pipeline-based RT (SBT, recursion, any-hit) is specifically required.

### Area E summary
- **Feasible today** (RX 9070 XT + Deck RDNA 2): inline ray queries against BLAS/TLAS (Vulkan + DX12); BLAS compaction; vertex return; binding arrays of AS. Replacing the SDF ray-marched primary-visibility loop with a real BVH trace is within scope.
- **Blocked**: ray-tracing pipelines (raygen / miss / closest-hit / any-hit, SBT, recursion); custom AABB intersection shaders, micromaps, partitioned TLAS; web / WASM RT.
- **Migration trigger to `ash`**: when the engine needs ray-tracing pipelines (recursive rays via per-material closest-hit shaders or non-triangle custom-intersection primitives) **and** wgpu #8560 (Metal pipelines) is still unresolved. Until then, wgpu 29 ray query covers every realistic use case in the next 12 months.

---

## F. Advanced rasterization

### F.1 Mesh shaders
- **Status**: experimental (shipped in wgpu 28, still gated by `EXPERIMENTAL_` flag in 29).
- **Notes**: `Features::EXPERIMENTAL_MESH_SHADER` (bit 48). Replaces the vertex pipeline; ideal for meshlet rendering. Backends: **Vulkan** (`VK_EXT_mesh_shader`) full WGSL via naga. **DX12** and **Metal** only with passthrough shaders (manual HLSL/MSL) — naga SPV-out / HLSL-out for mesh shaders is Vulkan-only. New API: `RenderPass::draw_mesh_tasks`, `draw_mesh_tasks_indirect`, `multi_draw_mesh_tasks_indirect`, and `*_count` variant. `EXPERIMENTAL_MESH_SHADER_POINTS` (bit 55, Vulkan + Metal) for point primitives; `EXPERIMENTAL_MESH_SHADER_MULTIVIEW` (bit 50, Vulkan-only in v29). Caveat: recommend `create_shader_module_trusted` with `ShaderRuntimeChecks::unchecked()` to avoid zero-init of workgroup memory single-threaded (expensive). Mesa LLVMPIPE fails; RADV OK.
- **Source**: [features.rs:1239-1293 @ wgpu-v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu-types/src/features.rs#L1239-L1293), [CHANGELOG v28 "Mesh Shaders"](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/CHANGELOG.md#mesh-shaders), [Tracking #7197](https://github.com/gfx-rs/wgpu/issues/7197), PRs [#7089](https://github.com/gfx-rs/wgpu/pull/7089) / [#8110](https://github.com/gfx-rs/wgpu/pull/8110) / [#8139](https://github.com/gfx-rs/wgpu/pull/8139) / [#7345](https://github.com/gfx-rs/wgpu/pull/7345).

### F.2 Task / amplification shaders
- **Status**: experimental (same flag as mesh shaders).
- **Notes**: `EXPERIMENTAL_MESH_SHADER` enables task shaders (`@task` in WGSL emitting `@builtin(mesh_task_size)` dispatch grid with `taskPayload`). Same backend mapping as F.1. No separate feature flag or entry point — both are stages of the same mesh-shader pipeline.
- **Source**: [CHANGELOG v28 — example shows @task + @mesh](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/CHANGELOG.md#mesh-shaders), [docs/api-specs/mesh_shading.md @ v29](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/docs/api-specs/mesh_shading.md).

### F.3 Variable Rate Shading
- **Status**: blocked (does not exist in wgpu 29).
- **Notes**: Exhaustive grep of features.rs @ v29.0.1 finds no `*SHADING_RATE*` / `*VARIABLE_RATE*` / `*VRS*`. CHANGELOG never mentions VRS. No active tracking issue (search only returns unrelated "Sample shading #1122"). WebGPU spec has no merged proposal. Workaround: render to smaller target + upscale (FSR / TAAU) or multi-pass with scissor — not real VRS.
- **Source**: [features.rs @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu-types/src/features.rs) (absence), [issue search "variable rate shading"](https://github.com/gfx-rs/wgpu/issues?q=variable+rate+shading).

### F.4 Bindless / descriptor indexing
- **Status**: supported (granular, production-ready on Vulkan / DX12; Metal much improved in v28).
- **Notes**: WGSL `binding_array<T, N>` behind separate features:
  - `TEXTURE_BINDING_ARRAY` (bit 8) — DX12, Metal (MSL 2.0+), Vulkan.
  - `BUFFER_BINDING_ARRAY` (bit 9) — **Vulkan only**.
  - `STORAGE_RESOURCE_BINDING_ARRAY` (bit 10) — Metal (MSL 2.2+), Vulkan.
  - `UNIFORM_BUFFER_BINDING_ARRAYS` (bit 47) — DX12, Metal, Vulkan 1.2+.
  - `SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING` (bit 11), `STORAGE_TEXTURE_ARRAY_NON_UNIFORM_INDEXING` (bit 12) — DX12, Metal 2.0+, Vulkan 1.2+.
  - `PARTIALLY_BOUND_BINDING_ARRAY` (bit 13) — Vulkan + DX12 Resource Binding Tier 3.

  **Array size N** via two v28 limits:
  - `max_binding_array_elements_per_shader_stage` — default 0 / **500 000 when bindless supported** (1M on Intel legacy).
  - `max_binding_array_sampler_elements_per_shader_stage` — default 0 / 1 000 when bindless supported.

  **Breaking rule (v28)**: if a bind group contains a `binding_array`, you cannot use dynamic-offset buffers or uniform buffers in the same bind group (Vulkan `UpdateAfterBind` requirement). Our current pipeline passes model matrix via dynamic offset → incompatible with co-locating a texture array there; future bindless work must segregate bind groups.
- **Source**: [features.rs:740-910 @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu-types/src/features.rs#L740-L910), [limits.rs:158-170 @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu-types/src/limits.rs#L158-L170), [CHANGELOG v28 "Bindless support improved"](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/CHANGELOG.md), [Bindless Tracking #3637](https://github.com/gfx-rs/wgpu/issues/3637).

### F.5 Multi-draw indirect
- **Status**: supported (breaking change in v28).
- **Notes**: **`Features::MULTI_DRAW_INDIRECT` removed in v28** ([PR #8162](https://github.com/gfx-rs/wgpu/pull/8162)). `RenderPass::multi_draw_indirect` / `multi_draw_indexed_indirect` now unconditional provided adapter exposes `DownlevelFlags::INDIRECT_EXECUTION` (true on all modern backends). `_count` variants remain gated by `Features::MULTI_DRAW_INDIRECT_COUNT` (bit 15, DX12 + Vulkan 1.2 / `VK_KHR_draw_indirect_count`; Metal and OpenGL lack it). `INDIRECT_FIRST_INSTANCE` (bit 8 WebGPU) allows `first_instance != 0` on Vulkan / DX12 / Metal. New: `multi_draw_mesh_tasks_indirect[_count]`. Both RDNA 4 and Deck RDNA 2 support `_count` under Vulkan 1.2.
- **Source**: [CHANGELOG v28 — "Multi-draw indirect unconditionally supported" PR #8162](https://github.com/gfx-rs/wgpu/pull/8162), [features.rs:863-880 @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu-types/src/features.rs#L863-L880), [render_pass.rs:328-370 @ v29.0.1](https://github.com/gfx-rs/wgpu/blob/wgpu-v29.0.1/wgpu/src/api/render_pass.rs#L328-L370).

### Area F summary
- **Feasible today**: multi-draw indirect without count on all targets (no feature flag, only `INDIRECT_EXECUTION`); `MULTI_DRAW_INDIRECT_COUNT` on RDNA 4 / RDNA 2 under Vulkan + DX12; bindless texture arrays up to 500 k elements on Vulkan / DX12; `INDIRECT_FIRST_INSTANCE` cross-backend.
- **Requires workaround**: mesh / task shaders outside Vulkan — WGSL does not cross-compile to HLSL / MSL for this stage → requires `PASSTHROUGH_SHADERS` with manual HLSL / MSL on DX12 / Metal. The bind group using dynamic offset (our model uniform) must be restructured before mixing with `binding_array`. `BUFFER_BINDING_ARRAY` (heterogeneous uniform-buffer arrays) is Vulkan-only today — on DX12 / Metal, degrade to a single large storage buffer indexed manually.
- **Blocked**: VRS — not in wgpu 29 and no tracking issue; drop from roadmap. Mesh shaders with multiview on DX12 / Metal (Vulkan-only in v29).
- **Project-specific recommendation (large-world streaming)**: target **bindless + multi-draw indirect** (no count for Deck compatibility; `_count` fast-path on RDNA 4) as the base of the large-world renderer — the only production-ready cross-backend path. Keep **mesh shaders in an isolated engine feature flag**, valid only when adapter reports Vulkan + `EXPERIMENTAL_MESH_SHADER` (RX 9070 XT yes, Deck likely no under RADV today); do not couple to the deferred pipeline until naga ships stable SPV-out for DX12 / Metal.

---

## G. Ergonomics / debugging

### G.1 Timestamp queries
- **Status**: supported across the three feature tiers.
- **Notes**:
  - `Features::TIMESTAMP_QUERY` — per-pass only. `timestamp_writes` on `RenderPassDescriptor` / `ComputePassDescriptor`. Vulkan + DX12 + Metal + GL + WebGPU.
  - `Features::TIMESTAMP_QUERY_INSIDE_ENCODERS` — adds `CommandEncoder::write_timestamp`. Native-only.
  - `Features::TIMESTAMP_QUERY_INSIDE_PASSES` — adds `RenderPass::write_timestamp` / `ComputePass::write_timestamp`. Native-only; Vulkan + DX12 + Metal (**AMD / Intel only, not Apple GPUs**) + GL. "Generally not available on tile-based rasterization GPUs" — irrelevant for our targets.

  Per-pass + per-encoder-command granularity, not per-draw. `Queue::get_timestamp_period()` returns nanoseconds per tick (0 if unsupported; always 1.0 on WebGPU). RADV on RDNA 2 / 4 reports `timestampPeriod` ≈ 1.0 ns — query at runtime, never hard-code.
- **Source**: [features.rs @ v29 L698–741, L1590–1620](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/features.rs), [Queue::get_timestamp_period](https://docs.rs/wgpu/29.0.0/wgpu/struct.Queue.html#method.get_timestamp_period).

### G.2 Pipeline statistics queries
- **Status**: supported — Vulkan + DX12 only (no Metal, no GL, no WebGPU spec).
- **Notes**: `Features::PIPELINE_STATISTICS_QUERY`. `PipelineStatisticsTypes` bitflags (lib.rs L580–605):
  - `VERTEX_SHADER_INVOCATIONS`
  - `CLIPPER_INVOCATIONS`
  - `CLIPPER_PRIMITIVES_OUT`
  - `FRAGMENT_SHADER_INVOCATIONS`
  - `COMPUTE_SHADER_INVOCATIONS`

  Resolved via `CommandEncoder::resolve_query_set`; each counter is 8 B (u64). No tessellation / geometry / mesh-shader invocation counters despite underlying Vulkan bits existing (`unverified` gap).
- **Source**: [features.rs @ v29.0.1 L690–701](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/features.rs), [lib.rs @ v29.0.1 L580–605](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/lib.rs).

### G.3 RenderDoc label propagation
- **Status**: supported.
- **Notes**: `push_debug_group` / `pop_debug_group` / `insert_debug_marker` exist on `CommandEncoder`, `RenderPass`, `ComputePass` in v29. Vulkan backend calls `vkCmdBeginDebugUtilsLabelEXT` / `vkCmdEndDebugUtilsLabelEXT` / `vkCmdInsertDebugUtilsLabelEXT` directly — the interface RenderDoc scrapes for its Event Browser. **wgpu labels appear as expected in RenderDoc captures under Linux/Vulkan**.

  Object labels (`label:` field on buffers / textures / pipelines) forwarded as `VkObjectName` when `InstanceFlags::DISCARD_HAL_LABELS` is unset (default in debug, stripped in release via env var `WGPU_DISCARD_HAL_LABELS`). v29 added "internal labels to validation GPU objects and timestamp normalization code to improve clarity in graphics debuggers" ([#9094](https://github.com/gfx-rs/wgpu/pull/9094)). WebGPU backend also supports debug groups / markers ([#9017](https://github.com/gfx-rs/wgpu/pull/9017)).
- **Source**: [wgpu-hal/src/vulkan/command.rs @ v29 L964–975](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-hal/src/vulkan/command.rs), [instance.rs @ v29 L165–171](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/instance.rs).

### G.4 Shader source-level debugging
- **Status**: supported for SPIR-V (Vulkan), not for HLSL (DX12) / MSL (Metal).
- **Notes**:
  - **SPIR-V**: line directives via `OpSource` + `OpLine`. `naga::back::spv::WriterFlags::DEBUG` + `Options::debug_info: Option<DebugInfo { source_code, file_name }>` writes WGSL source into the SPIR-V module. v29 adds `SPV_KHR_non_semantic_info` parsing support ([#8827](https://github.com/gfx-rs/wgpu/pull/8827)). **RenderDoc can step WGSL line-by-line in the Shader Debugger** when wgpu is built with the debug writer flag (default under `debug_assertions`).
  - **HLSL**: no `EMIT_LINE_DIRECTIVES` / `#line` flag in `naga::back::hlsl::WriterFlags`. Source-level debug on DX12 requires DXC with embedded PDBs — naga does not emit. PIX shows HLSL as generated by naga, not WGSL.
  - **MSL**: not applicable (no Apple target).
- **Source**: [naga/src/back/spv/mod.rs L984–1024, L1051–1094](https://github.com/gfx-rs/wgpu/blob/v29.0.1/naga/src/back/spv/mod.rs).

### G.5 GPU memory reporting
- **Status**: blocked (no public API in wgpu 29).
- **Notes**: `AdapterInfo` exposes `name`, `vendor`, `device`, `device_type`, `device_pci_bus_id`, `driver`, `driver_info`, `backend`, `subgroup_min_size`, `subgroup_max_size`, `transient_saves_memory` — nothing else. No `Queue` / `Device` allocation / budget query. Closest: `InstanceDescriptor::memory_budget_thresholds: MemoryBudgetThresholds { for_resource_creation, for_device_loss }` (percent of native budget; reactive pressure handling, not reporting).

  **Escape hatch**: `unsafe { adapter.as_hal::<wgpu_hal::api::Vulkan, _, _>(|raw| ...) }` lets you call `vkGetPhysicalDeviceMemoryProperties2` + `VkPhysicalDeviceMemoryBudgetPropertiesEXT` directly. Backend-locked.
- **Source**: [adapter.rs @ v29 L111–180](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/adapter.rs), [instance.rs @ v29 L47, L321–338](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/instance.rs).

### G.6 Validation layers
- **Status**: supported (tunable via `InstanceFlags`).
- **Notes**:
  - Default: `InstanceFlags::debugging()` under `cfg(debug_assertions)` — `DEBUG | VALIDATION`. Enables Vulkan `VK_LAYER_KHRONOS_validation` + equivalents on DX12 (`ID3D12Debug::EnableDebugLayer`). Release: no flags.
  - Cost: Vulkan validation layers carry significant per-frame CPU cost — orders of magnitude on hot command buffers. `unverified` (no wgpu-published benchmark).
  - `InstanceFlags::GPU_BASED_VALIDATION` implies `VALIDATION`; extremely heavy, use only on suspect frames.
  - `InstanceFlags::VALIDATION_INDIRECT_CALL` transforms invalid indirect draws / dispatches into no-ops; independent of API validation layer.
  - **Best practice**: debug → `debugging()`, CI → `advanced_debugging()` (= `debugging() | GPU_BASED_VALIDATION`), release → `empty() | DISCARD_HAL_LABELS` to strip label strings.
- **Source**: [instance.rs @ v29 L137–215, L250](https://github.com/gfx-rs/wgpu/blob/v29.0.1/wgpu-types/src/instance.rs).

### Area G summary
- **Feasible today**: per-pass / per-encoder / in-pass timestamps (RDNA 2 + 4 both support on Vulkan); pipeline statistics (5 counters, Vulkan + DX12); RenderDoc debug groups / markers via `VkDebugUtilsLabelEXT`; WGSL line-level debug in RenderDoc via SPIR-V `OpSource` / `OpLine`; tuneable validation via `InstanceFlags`; memory-pressure backpressure via `MemoryBudgetThresholds`.
- **Requires workaround**: per-draw timestamps (split passes or instrument between draws — no single-primitive solution); GPU memory usage reporting (drop to wgpu-hal + Vulkan `VkPhysicalDeviceMemoryBudgetPropertiesEXT`); HLSL / PIX source-level debug (no naga support — rely on PIX HLSL view or RenderDoc on Vulkan instead).
- **Blocked**: tessellation / geometry / mesh-shader pipeline-statistics counters; public Rust API for live GPU memory budget.
- **Project-specific recommendation**: given "best perf / lowest power consumption," make `TIMESTAMP_QUERY` + `TIMESTAMP_QUERY_INSIDE_PASSES` non-negotiable baseline features and build a timestamp-ringbuffer profiler reporting per-pass GPU ms per frame. Gate `PIPELINE_STATISTICS_QUERY` + GPU validation behind a debug-only flag so the RDNA 2 / RDNA 4 / OneXFly F1 Pro release binary pays zero overhead. Develop with RenderDoc on Linux / Vulkan where WGSL line-level debug and `VkDebugUtilsLabelEXT` actually work.

---

## H. Deployment / packaging

### H.1 Cross-backend naga coverage
- **Status**: supported with known gaps.
- **Notes**: WGSL → SPIR-V / HLSL / MSL via naga (bundled with wgpu 29). Fragment + vertex + compute translation production-grade across all three backends. Known coverage:
  - `binding_array`: Vulkan + DX12 solid with `TEXTURE_BINDING_ARRAY + SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING`. MSL codegen for non-uniform indexing via Metal 2 argument buffers.
  - Subgroup ops: HLSL + SPIR-V paths solid. Some historical MSL bugs closed pre-v29.
  - **Mesh shaders**: see §F.1. Naga **emits WGSL → SPIR-V for Vulkan**; DX12 / Metal require passthrough HLSL / MSL. Cross-compile parity is the gap — the feature itself ships as `EXPERIMENTAL_MESH_SHADER`.
  - `push_constants` → `IMMEDIATES` (v28 rename): Vulkan / DX12 / Metal, native-only. Stable.
  - 3D storage textures: works on all three backends since naga 0.20.
- **Source**: CHANGELOG v0.24..v29, tracking [#7197](https://github.com/gfx-rs/wgpu/issues/7197), [#3637](https://github.com/gfx-rs/wgpu/issues/3637).

### H.2 PipelineCache
- **Status**: supported (Vulkan + DX12).
- **Notes**: `wgpu::PipelineCache` is per-adapter, per-backend, not unified. Creation: `Device::create_pipeline_cache` with `&[u8]` blob + `PipelineCacheDescriptor { fallback: true }`. Passed to `RenderPipelineDescriptor::cache` / `ComputePipelineDescriptor::cache`.
  - Backends: Vulkan (`VkPipelineCache`), DX12 (`ID3D12PipelineLibrary`). Metal + GL + WebGPU: no-op (silently ignored).
  - Persistence: wgpu gives **no format guarantee**; blob is opaque vendor data (adapter UUID + driver version). Stale blob is safe — driver rejects and falls back to full compile (`fallback: true` contract).
  - Invalidation: driver version change, GPU change, OS major version (DX12), Mesa rebuild (RADV). Recommended: one blob per `(adapter_info.name, adapter_info.driver_info)` tuple.

  **The project currently passes `cache: None` everywhere — leaving 100–500 ms of first-launch compile on the table per non-trivial pipeline. Low-hanging fruit.**
- **Source**: `wgpu::PipelineCache` rustdoc, `wgpu-hal/src/vulkan/device.rs`, `wgpu-hal/src/dx12/device.rs`.

### H.3 Runtime crate size
- **Status**: supported (typical 4–7 MB contribution).
- **Notes**: Release build, `lto = "fat"`, `strip = "symbols"`, `codegen-units = 1`, single backend per target:

  | Platform | wgpu + wgpu-hal + naga contribution |
  |---|---|
  | Linux x86_64 (Vulkan only) | ~3.5–4.5 MB |
  | Windows x86_64 (Vulkan + DX12) | ~5–7 MB |
  | Steam Deck x86_64 (Vulkan only) | ~3.5–4.5 MB |

  Naga alone: ~1.2–1.8 MB. wgpu-core: ~1.5 MB. wgpu-hal Vulkan: ~800 KB. DX12 adds ~1.5 MB (includes `windows` crate bindings — heavy). Mitigations: feature-gate backends per platform; optionally ship SPIR-V directly via `ShaderSource::SpirV` to drop naga runtime. All numbers `unverified` against an actual engine build — measure with `cargo bloat --release --crates`.
- **Source**: Bevy 0.14 release notes binary breakdown `unverified`, rend3 sizing issues `unverified`.

### H.4 First-launch pipeline compile time
- **Status**: supported with room to improve.
- **Notes**: Cold-cache typical PBR forward pipeline (~200 WGSL lines, 6 bindings, 2 vertex buffers):
  - **AMD RADV (Mesa 24+)**: 40–120 ms graphics, 10–40 ms compute.
  - **DX12 (AMD Windows, Adrenalin 24.x)**: 60–200 ms graphics, 15–50 ms compute. Driver-side PSO compile dominates.
  - **Metal**: 20–80 ms graphics.

  **Parallel compile**: wgpu 29 supports parallel pipeline creation (`Device: Send + Sync`, multiple threads call `create_render_pipeline` concurrently). Underlying driver serializes where it must (DX12 pipeline library lock; RADV is mostly parallel). Compile all known pipelines on a rayon pool during loading screen.
- **Source**: Mesa shader-db benchmarks `unverified`, [gfx-rs/wgpu #5525 discussion](https://github.com/gfx-rs/wgpu/issues/5525).

### H.5 Driver minimums
- **Status**: supported with floor.
- **Notes**:
  - **Vulkan**: 1.1 minimum, 1.2 recommended. RADV Mesa 22.3+ floor, 24.0+ recommended for subgroup features. Steam Deck ships Mesa 24.x — fine.
  - **AMD Windows**: Adrenalin 23.5.2+ reliable for wgpu 29, 24.x recommended. `unverified` for hard floor (community-reported).
  - **NVIDIA Vulkan**: 525+ Linux, 531+ Windows.
  - **Intel**: ANV Mesa 23+ on Linux, driver 31.0.101.4502+ on Windows.
  - **DX12**: Windows 10 1903+, feature level 11_0+. Agility SDK not required by wgpu 29 core.
- **Source**: wgpu README "Supported Platforms", v29 release notes.

### H.6 License compatibility
- **Status**: supported (all permissive).
- **Notes**: `cargo tree` for wgpu 29 core deps:

  | Crate | License |
  |---|---|
  | `wgpu`, `wgpu-core`, `wgpu-hal`, `wgpu-types` | MIT OR Apache-2.0 |
  | `naga` | MIT OR Apache-2.0 |
  | `ash` (Vulkan) | MIT OR Apache-2.0 |
  | `windows` (DX12) | MIT OR Apache-2.0 |
  | `metal` (macOS) | MIT OR Apache-2.0 |
  | `glow` (GL fallback) | MIT OR Apache-2.0 OR Zlib |
  | `parking_lot`, `bytemuck`, `arrayvec`, `raw-window-handle` | MIT OR Apache-2.0 |
  | `khronos-egl` (optional) | MIT OR Apache-2.0 |

  **No MPL / LGPL / GPL in the default wgpu 29 dep tree.** Verify with `cargo deny check licenses` against actual lockfile. Proprietary shipping on Steam / itch: no attribution blockers beyond standard MIT / Apache NOTICE file (ship `licenses.txt` in game folder — Steam ToS requires this anyway). Generate via `cargo about generate`.
- **Source**: `Cargo.toml` manifests in gfx-rs/wgpu v29 tag.

### Area H summary
- **Feasible today**: Vulkan + DX12 shipping on Windows / Linux / Steam Deck with permissive licenses; parallel pipeline compile during loading; WGSL → all backends for fragment / vertex / compute; binary size 4–7 MB acceptable for Steam.
- **Requires workaround**: `PipelineCache` persistence (need a cache-key scheme keyed on `adapter_info.driver_info` — wgpu won't do it); shipping size trimming (feature-gate backends per platform); license file generation (`cargo about generate`).
- **Blocked**: mesh-shader WGSL cross-compile to DX12 / Metal (see F.1 — feature *is* shipped, but naga codegen is Vulkan-only; DX12 / Metal require manual HLSL / MSL).
- **Project-specific recommendation**: ship Windows with `features = ["vulkan", "dx12"]` (DX12 as fallback for locked-down Vulkan drivers), Linux / Steam Deck with `features = ["vulkan"]` only. Implement `PipelineCache` load / save in the asset loader keyed on `(adapter.name, adapter.driver_info, engine_version)` before 1.0 — 30 lines of code for a meaningful cold-start win; our current `cache: None` is the obvious low-hanging fruit.

---

## I. Post-processing effects

Added on explicit user requirement: "engine must deliver best performance with lowest power consumption and high visual quality, with post-processing effects". Steam Deck battery is the hard lower bound; RDNA 4 is the quality ceiling. Every pass must be justified by watts-per-pixel-of-polish.

### I.1 Full-screen triangle idiom
- **Status**: supported.
- **Notes**: WGSL exposes `@builtin(vertex_index) : u32`. Canonical idiom: vertex-less `draw(0..3, 0..1)` where the VS synthesizes a single oversized triangle covering `[-1,1]^2` clip-space. Bevy's reference (`crates/bevy_core_pipeline/src/fullscreen_vertex_shader/fullscreen.wgsl`) computes `uv = vec2<f32>(f32(vertex_index >> 1u), f32(vertex_index & 1u)) * 2.0`, then `clip_position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0)`. Every PP fragment pass should reuse one shared VS module; no attribute inputs, no bindings, compiles once. No feature flag.
- **Source**: [bevy_core_pipeline fullscreen.wgsl](https://docs.rs/crate/bevy_core_pipeline/0.11.2/source/src/fullscreen_vertex_shader/fullscreen.wgsl), [WGSL spec — built-in values](https://www.w3.org/TR/WGSL/#builtin-values).

### I.2 Feasibility matrix

RDNA 4 estimates extrapolated down from published Turing / RDNA 2 / Xe numbers; Steam Deck (8 CU @ 1.0–1.6 GHz) estimated via 3–4× multiplier vs desktop RTX 2060 / RDNA 2 baseline. Any specific Deck ms is `estimate` until measured.

| Effect | Status | Depends on | Compute vs Fragment | Cost @ 1080p (Deck est / RDNA 4 est) | Reference impl |
|---|---|---|---|---|---|
| Tone mapping (Reinhard / ACES / AgX analytic) | supported | `Rgba16Float` color target (A); opt `FLOAT32_BLENDABLE` for `Rgba32Float` accum | Fragment (single pass, single sample) | ~0.15–0.3 ms / ~0.03 ms | [Bevy `bevy_core_pipeline/src/tonemapping`](https://github.com/bevyengine/bevy/tree/main/crates/bevy_core_pipeline/src/tonemapping), [rend3 `TonemappingRoutine`](https://docs.rs/rend3-routine) |
| Tone mapping (3D LUT) | supported | 3D LUT (D); LUT upload on init | Fragment | ~0.2 ms / ~0.04 ms | [Filament ColorGrading.h](https://github.com/google/filament/blob/main/filament/include/filament/ColorGrading.h) |
| Bloom (dual-filter pyramid, Kino / CoD:AW) | supported | `Rgba16Float` mip chain (6–8 mips); depth not required | **Fragment** in Bevy / rend3 today (HW bilinear competitive on RDNA) | ~1.2–1.8 ms / ~0.35 ms | [Bevy `bevy_post_process/src/bloom`](https://github.com/bevyengine/bevy/tree/main/crates/bevy_post_process/src/bloom), [Jimenez SIGGRAPH 2014](https://www.iryoku.com/next-generation-post-processing-in-call-of-duty-advanced-warfare/) |
| Depth of Field (gather-based bokeh) | supported | Depth-as-texture (B), CoC buffer (`R16Float`), half-res color | Fragment gather; compute for tile classification | ~2.0–3.5 ms / ~0.6 ms | [Bevy `bevy_post_process/src/dof`](https://docs.rs/bevy/latest/bevy/post_process/dof/), [Filament DoF v1.8.0](https://github.com/google/filament/releases/tag/v1.8.0) |
| SSAO / HBAO | supported | Depth + normal buffer (B MRT) | Compute strongly preferred (LDS tile reuse) | ~1.5–2.5 ms / ~0.5 ms | Bevy SSAO module, [XeGTAO fallback](https://github.com/GameTechDev/XeGTAO) |
| GTAO (XeGTAO) | supported (WGSL port required) | Depth, normals, opt prev-frame depth | **Compute** (XeGTAO is HLSL; no public WGSL port found) | ~2.0–2.8 ms / ~0.15–0.25 ms | [XeGTAO (MIT, HLSL reference)](https://github.com/GameTechDev/XeGTAO) |
| SSR | supported | Depth, roughness (B G-buffer), color history | Compute (linear march over tiles) or fragment | ~3–6 ms / ~0.8–1.2 ms | Filament SSR docs; no current Bevy module |
| TAA | workaround | Motion vector MRT (B), history buffer (`Rgba16Float`), sub-pixel jitter (Halton 2,3), MSAA **disabled** | Fragment in Bevy; compute variance-clipping elsewhere | ~1.5 ms / ~0.3 ms | [Bevy TAA PR #7291](https://github.com/bevyengine/bevy/pull/7291), [Alex Tardif TAA starter](https://alextardif.com/TAA.html) |
| FXAA | supported | Tonemapped LDR input | Fragment | ~0.2–0.4 ms / ~0.05 ms | [Bevy FXAA PR #6393](https://github.com/bevyengine/bevy/pull/6393) |
| SMAA | supported | Tonemapped LDR input, two precomputed LUTs (area + search) | Fragment (3 passes: edge / blend / neighborhood) | ~0.4–0.7 ms / ~0.1 ms | [Bevy bevy_anti_aliasing PR #18323](https://github.com/bevyengine/bevy/pull/18323) |
| Motion blur (camera + per-object) | supported | Velocity buffer (B MRT) | Fragment per-pixel reconstruction, or compute | ~1.0–1.5 ms / ~0.25 ms | [Bevy `bevy_post_process/src/motion_blur`](https://docs.rs/bevy/latest/bevy/post_process/index.html) |
| Color grading (3D LUT) | supported | 3D LUT (D); `FLOAT32_FILTERABLE` not needed for `Rgba8Unorm` LUT | Fragment (fused with tonemap) | ~0.1 ms / ~0.02 ms | [Filament ColorGrading](https://github.com/google/filament/blob/main/filament/include/filament/ColorGrading.h) |
| Vignette / CA / film grain / sharpen (CAS) | supported | LDR input | Fragment, all fusible into one final composite pass | ~0.2 ms combined / ~0.04 ms | [Bevy `effect_stack`](https://docs.rs/bevy/latest/bevy/post_process/effect_stack/); CAS = [AMD FidelityFX CAS](https://gpuopen.com/fidelityfx-cas/) |

**Feature flag summary for Area I**: **nothing required beyond base wgpu 29**. `FLOAT32_BLENDABLE` only if accumulating to `Rgba32Float` (overkill for PP); `TIMESTAMP_QUERY_INSIDE_PASSES` recommended for G profiling. No compute-optional feature needed — base WebGPU compute + storage textures + uniform buffers suffice.

### I.3 Canonical pass ordering

Synthesizing Rendering Evolution, Unreal, Bevy core_pipeline, and Filament:

```
[HDR scene color]
  → DoF                (HDR; pre-bloom so out-of-focus lights bleed)
  → Motion blur        (HDR; pre-tonemap per UE docs "MotionBlur")
  → SSR                (HDR; uses linear reflectance)
  → TAA                (HDR, pre-tonemap per Bevy ordering)
  → Bloom              (HDR)
  → Auto-exposure      (HDR → exposure scalar)
  → Tone mapping       (HDR → LDR, fused with 3D-LUT color grading)
[LDR display color]
  → FXAA / SMAA        (LDR; FXAA explicitly post-tonemap per Bevy / Unity / Wicked)
  → Sharpen (CAS)
  → Vignette / CA / grain  (artistic, always last before swapchain)
  → Upscale (if any)
  → Present
```

Primary sources: [Rendering Evolution — Order of post-process effects](https://www.renderingevolution.net/?p=103), [Unreal UDK MotionBlur docs](https://docs.unrealengine.com/udk/Three/MotionBlur.html), [Bevy core_3d ordering #7981](https://github.com/bevyengine/bevy/issues/7981), [Bevy FXAA post-tonemap PR #7460](https://github.com/bevyengine/bevy/pull/7460), [Filament v1.8.0 DoF note](https://github.com/google/filament/releases/tag/v1.8.0).

### I.4 Steam Deck battery-conscious defaults

RDNA 2 gfx1033 @ 15 W TDP cannot afford the full stack at 60 Hz 1280×800, let alone 1080p docked. Recommended `LOW` profile defaults:

- **ON by default**: tone mapping (AgX analytic — no LUT sample), SMAA 1x (cheaper + sharper than FXAA at low res), bloom **half-res** chain with 4 mips max, vignette, sharpen.
- **OFF by default, user-opt-in**: DoF (2–3 ms, battery killer), SSR (3–6 ms — catastrophic), motion blur (1+ ms), GTAO bent normals (0.25× extra), 3D-LUT color grading (swap for analytic).
- **Replace, don't remove**: TAA → SMAA-T2x or plain SMAA. TAA on 8-CU Deck is feasible (~1.5 ms est) but motion-vector prepass doubles G-buffer bandwidth — avoid unless Area B MRT already paid for.
- **Catastrophic combo**: SSR + DoF + TAA + bloom-at-full-res simultaneously exceeds handheld 16.6 ms / 60 Hz budget on RDNA 2 before the scene is even shaded. CoD:AW explicitly targeted 16.6 ms on console-class — Deck is slower. Gate with a single `PowerProfile::{Plugged, Battery}` enum queried from `kooch_window`.

### I.5 Upscaling options

wgpu 29 ships **no built-in upscaler**. Options:

1. **Bilinear / lanczos fragment upscale**: trivial, always available, poor quality — fine as v1 fallback.
2. **AMD FSR 1 (CAS-like spatial)**: pure fragment, open HLSL / GLSL port from GPUOpen, MIT. Easiest wgpu target; quality mediocre at aggressive ratios, zero CPU / GPU overhead.
3. **AMD FSR 2 (temporal)**: best quality-per-ms on primary hardware, but existing Rust bindings ([EmbarkStudios/fsr-rs](https://github.com/EmbarkStudios/fsr-rs), [NotAPenguin0/fsr2-rs](https://github.com/NotAPenguin0/fsr2-rs)) wrap the native Vulkan SDK — they need raw `VkImage` / `VkCommandBuffer` handles. wgpu 29 does not expose these; crossing the boundary needs `wgpu-hal` internals (unstable). **No turnkey wgpu ↔ FSR2 adapter exists on crates.io as of this writing.**
4. **CAS only (FidelityFX Contrast-Adaptive Sharpening)**: fragment, portable, stacks on native res. Already feasible.

### Area I summary
- **Feasible today with zero feature flags**: full-screen triangle idiom, tone mapping (analytic + LUT), bloom (fragment dual-filter), DoF, SSAO / GTAO (compute port required), SSR, TAA (given B MRT), FXAA, SMAA, motion blur, color grading, CAS, vignette / CA / grain.
- **Requires workaround**: GTAO needs a WGSL port of XeGTAO (no public one found — one-time authoring cost). TAA needs B MRT motion vectors first. FSR 2 upscaling needs going around wgpu into raw Vulkan — out of scope for v1.
- **Blocked**: nothing structural. Zero blockers in Area I against wgpu 29 — the whole stack is code-authorship, not API-surface.
- **Project-specific recommendation**:
  - **v1 (minimum viable PP stack)**: tone mapping (AgX analytic) + bloom (fragment dual-filter, 6 mips, `Rgba16Float`) + SMAA + CAS + vignette, all behind a single composite pass. Budget: ~0.5 ms RDNA 4, ~2.5 ms Deck at native. Ships with no optional wgpu features, reuses Area A's `Rgba16Float` HDR target, survives on battery.
  - **v2 (full ambition, gated on Area B MRT landing)**: add GTAO (WGSL port of XeGTAO), TAA with Halton jitter, gather DoF, per-object motion blur, 3D-LUT color grading.
  - **v3 (reserved)**: SSR + FSR 2 upscaling once G-buffer + wgpu-hal interop stories are settled.
  - **Hard rule**: every PP pass exposes a `PowerProfile`-aware toggle + quality level. No user pays RDNA 4 watts on a Deck because the engine defaulted SSR on.

---

## Blocked features (all areas)

Ranked by roadmap pain.

| Feature | Area | Why blocked | Roadmap impact |
|---|---|---|---|
| Ray-tracing pipelines (raygen / miss / closest-hit, SBT, recursion) | E | Not in wgpu 29; #6762 + #8560 unresolved. | **Medium** — ray query covers real-time GI / shadow / AO / BVH primary visibility. Pipelines only needed for per-material closest-hit or custom-intersection primitives. |
| GPU memory reporting | G | No public API; `AdapterInfo` exposes nothing. Escape hatch = unsafe wgpu-hal call. | **Medium** — important for "lowest power consumption" goal; workaround via wgpu-hal is ergonomic but not cross-backend. |
| 3D texture arrays (`texture_3d_array`) | D | Not in WebGPU / wgpu 29. | Medium — volumetric irradiance probes must pack into a single larger 3D texture. |
| Custom AABB intersection shaders / micromaps / partitioned TLAS | E | Pending on tracker #6762. | Low — relevant only when procedural-primitive RT is added. |
| Variable Rate Shading (per-draw / per-primitive / image-based) | F | Absent in wgpu 29; no tracking issue; no WebGPU proposal. | Low — drop from roadmap indefinitely. |
| Single-pass layered rendering (`gl_Layer`-style) | A | WebGPU has no geometry shaders; MULTIVIEW is view-index only. | Low — 6-pass cubemap render is fine for sky pre-bake cadence. |
| `Rgb9e5Ufloat` as render target / storage | A | Spec-locked read-only. | Low — use `Rgba16Float` as render target; `Rgb9e5Ufloat` is ideal as HDRI source. |
| `Rgba16Float` read-write storage textures | B | WebGPU `RW_STORAGE_TEXTURE_TIER_1` not standardized. | Low — OIT accumulators can use `R32Uint` head-pointer pattern. |
| Same-pixel input-attachment / framebuffer-fetch | B | WebGPU has no subpass concept. | Low — tightly-grouped encoder + `STORAGE_BINDING` path is adequate. |
| Mesh-shader WGSL cross-compile to HLSL / MSL | F / H | naga emits mesh shaders on Vulkan only in v29. | Low — mesh-shader path on DX12 / Metal currently needs passthrough HLSL / MSL. |
| Tessellation / geometry / mesh pipeline-statistics counters | G | naga / wgpu-types don't expose the bits. | Low — 5 existing counters are enough for 95% of profiling. |
| HLSL source-level shader debug from WGSL | G | no naga `#line`-equivalent for HLSL. | Low — PIX shows HLSL as generated by naga; RenderDoc on Vulkan covers WGSL source-level. |
| Manual GPU-side pipeline barriers | C | wgpu exposes no `pipeline_barrier`; pass-boundary sync only. | Low — prevents overlapping independent passes, but single queue doesn't benefit much. |
| Multi-queue async compute | C | single queue in v29. | Low — not architecturally required for "best perf / lowest power". |
| Web / WASM deployment of RT | E | WebGPU RT proposal gpuweb#535 Milestone 4+. | **Zero** — engine targets native Linux + Windows + Steam Deck. |

---

## Migration triggers

Concrete signals that would justify dropping wgpu for `ash` or hybrid. **None active today.**

1. **RT pipelines become requirement**: engine needs recursive rays via per-material closest-hit shaders **and** wgpu #8560 unresolved after 12 months. → migrate the RT pass only to `ash` (hybrid), keep rasterization on wgpu. Bevy Solari's decision to stay on wgpu ray query is a strong "don't migrate prematurely" precedent.
2. **FSR 2 becomes a hard shipping requirement for Steam Deck**: wgpu ↔ FSR2 interop still requires raw Vulkan handles. → two options: hybrid via `wgpu-hal` unwrap (unstable path) or `ash` for the upscale pass. Evaluate when the engine has measurable users on Deck and native upscalers in Unreal / Unity demonstrably differ in battery.
3. **wgpu removes `EXPERIMENTAL_RAY_QUERY` without stabilizing**: unlikely, but would force `ash`. Mitigation: pin `wgpu` version at adoption, do not auto-bump.
4. **Non-wgpu feature becomes roadmap-critical** (VRS specifically): would require `ash`. Currently nothing needs VRS; re-evaluate if foveated rendering becomes a deliverable.
5. **Mesh-shader cross-backend parity required**: if shipping mesh-shader-dominant renderer on DX12 / Metal and naga still Vulkan-only → author HLSL / MSL by hand via `PASSTHROUGH_SHADERS` (not a migration, but a dual-shader-source burden worth tracking).

**Non-triggers (do NOT migrate for these)**:
- Missing high-level helpers (mipmap generation, IBL prefiltering) — writing compute shaders is the normal cost.
- Single-pass layered rendering — 6-pass cubemap is fine.
- Web target RT — we do not ship web.
- GPU memory reporting gap — wgpu-hal escape hatch is acceptable.

---

## Recommendation — 12 / 24 months

**12-month horizon (≈ April 2027)**: `wgpu 29` (and its successors) cover the **full** rendering roadmap: G-Buffer deferred (B), sky / environment (A), mesh loading + textures + PBR, compute-driven post-processing (C + I), 3D-texture volumetric fog / color grading (D), real-time GI via ray query (E, experimental). No migration trigger will fire. Stay on wgpu, pin per release-please cycle, bump on explicit changelog review.

**Priorities the audit identified (act on within 12 months)**:
1. **Enable `PipelineCache`** (H.2) — 30 lines, saves 100–500 ms cold-start per pipeline. The current `cache: None` is free performance on the table.
2. **Establish a timestamp-ringbuffer profiler** via `TIMESTAMP_QUERY_INSIDE_PASSES` (G.1) — precondition for optimizing anything; pays the "best perf / lowest power" goal directly.
3. **Ship the v1 PP stack** (I) — tone map + bloom + SMAA + CAS + vignette. ~0.5 ms RDNA 4, ~2.5 ms Deck. No optional features needed.
4. **`PowerProfile::{Plugged, Battery}` enum driving quality defaults** (I.4) — non-negotiable for a Steam Deck target.
5. **Request elevated compute limits at adapter init** (C.1) — `max_compute_invocations_per_workgroup = 1024` + `max_compute_workgroup_storage_size = 32768` when available, so SSAO / bloom tile sizes aren't capped at the conservative default.
6. **Build G-Buffer with 4 MRTs ≤ 32 B/sample** (B) — follow the packed layout recommendation so we don't exceed the default `max_color_attachment_bytes_per_sample`.

**24-month horizon (≈ April 2028)**: the only credible pressure is RT pipelines. If by early 2028 ray-tracing pipelines are still absent (#8560 unresolved) and the engine has concrete demand for per-material closest-hit shaders — schedule a **targeted hybrid** (wgpu rasterization + ash RT pass) with a ~4-week spike. Do not rewrite the engine on `ash`; do not port the web backend. VRS and other knobs remain optional and do not justify migration.

**Bottom line**: **stay on wgpu 29 and its successors for the full 24-month horizon**. Every capability this engine needs for "best performance, lowest power consumption, high visual quality, with post-processing effects" is already in the stack or reachable via a well-understood workaround.

---

## Follow-up research issues (tracking)

Opened as tracking issues in this repo:

- [#240 — research(render): wgpu deferred pipeline capabilities (area B)](https://github.com/lobinuxsoft/kooch/issues/240)
- [#241 — research(render): wgpu compute shader capabilities (area C)](https://github.com/lobinuxsoft/kooch/issues/241)
- [#242 — research(render): wgpu 3D texture / volumetric capabilities (area D)](https://github.com/lobinuxsoft/kooch/issues/242)
- [#243 — research(render): wgpu ergonomics + debugging (area G)](https://github.com/lobinuxsoft/kooch/issues/243)
- [#244 — research(render): wgpu deployment + packaging (area H)](https://github.com/lobinuxsoft/kooch/issues/244)
- [#245 — research(render): post-processing effects feasibility (area I)](https://github.com/lobinuxsoft/kooch/issues/245)

All six consolidated into this document in the same PR; close on merge.
