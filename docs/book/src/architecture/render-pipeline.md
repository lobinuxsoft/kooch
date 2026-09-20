# Render Pipeline

Kóoch renders through a **GPU-driven meshlet pipeline**, Nanite-style: the
CPU uploads a flat array of instances and dispatches, and every decision
about *what to draw* — frustum, backface, occlusion, level of detail — is
taken on the GPU by a compute shader reading that array.

The CPU never walks a scene graph deciding what is visible. That is the
whole point, and it is what "GPU-driven" means here.

> This page describes what the code does today. Where something is
> missing the page says so and links the issue.

## A frame is a list of views

`MeshletRenderStage` owns one geometry pool and a `SlotMap<ViewId,
MeshletView>`. Each view has its own render targets, its own cull state
and its own camera; the pool, the instance buffer and the pipelines are
shared.

That split is deliberate and was not always true. Cull state is per view
**by definition** — what survives a frustum test depends on where the
camera is — and sharing it across views produces an over-cull that only
appears once a second view exists, or once shadow cascades do, where it
reads as "the shadows are wrong" rather than as a shared-state bug.

Two views run today: the editor's **View** panel and its **Game** panel.

**Shadow cascades did not become views**, which was the plan when this
page was written. They record inside the stage instead, against the
unjittered camera and with their own bounded projection, because a
cascade shares the pool and the instance buffer but wants none of a
view's render targets. Virtual-shadow-map pages
([#477](https://github.com/lobinuxsoft/kooch/issues/477)) may still be
the case that makes a view the right shape.

Each view records **and submits** its own command encoder. Several
per-frame buffers are shared across views on exactly that basis: a write
followed by a submit is ordered on the queue, so view B's camera cannot
reach view A's pass.

## The frame, pass by pass

Which path runs depends on one capability: 64-bit texture atomics. The
device either has `TEXTURE_INT64_ATOMIC` + `SHADER_INT64` +
`SHADER_INT64_ATOMIC_MIN_MAX` or it does not.

The node labels below are the **GPU scope names a capture prints**, so a
flamegraph and this diagram can be read side by side. Anything not named
here does not have a timer on it.

```mermaid
flowchart TD
    START([Frame begins]) --> EXTRACT[CPU: walk the ECS<br/>MeshRenderer + GlobalTransform → instances<br/>lights → Inti's GPU buffer]
    EXTRACT --> UPLOAD[Upload instances, grow buffers to fit]
    UPLOAD --> SHADOWS["shadows<br/>4 cascade culls + rasters,<br/>plus a cube face per shadowed point light"]
    SHADOWS --> GRID["cluster grid<br/>the froxel light index — 4 passes, two of them draws"]
    GRID --> R64{64-bit texture<br/>atomics?}

    R64 -- yes --> A0["cull: one thread per instance-meshlet<br/>frustum · backface cone · LOD chain descent"]

    subgraph FUSED["raster + shade — one fused scope, timed as a whole"]
        direction TB
        A1[Clear the R64 visibility buffer] --> A2["Raster: draw_indirect the survivors<br/>fragment does atomicMax(depth &lt;&lt; 32 | ids)"]
        A2 --> MV["motion vectors<br/>previous clip position, unjittered camera"]
        MV --> SH{"compute<br/>shading?"}
        SH -- yes --> A4["shade: compute — or (half rate)<br/>one dispatch → Inti, into an HDR target"]
        SH -- "no, the default" --> A5["shade: fragment<br/>one fullscreen pass per material, depth-tested Equal"]
        A4 --> UP["shade: upsample<br/>only when the rate is half"]
        UP --> TAA["taa / sgsr2<br/>the temporal resolve — off by default, #481"]
        TAA --> TM["tonemap<br/>HDR radiance → display-referred"]
        TM --> RCAS["rcas<br/>sharpening, after the curve — off by default"]
    end

    A0 --> A1

    R64 -- no --> B0["cull + raster A against last frame's Hi-Z"]
    B0 --> B2["hi-z build: SPD pyramid"]
    B2 --> B3["cull + raster B: what pass A occluded"]
    B3 --> B5["shade: one compute dispatch → Inti<br/>no motion vectors, no TAA on this path"]

    RCAS --> SKY["sky"]
    A5 --> SKY
    B5 --> SKY
    SKY --> BLIT["blit the stage's colour over the sky"]
    BLIT --> PRESENT([Present])

    style A2 fill:#1e5f3a,stroke:#4dbe8f,color:#fff
    style A4 fill:#5f3a1e,stroke:#be8f4d,color:#fff
    style A5 fill:#5f3a1e,stroke:#be8f4d,color:#fff
    style B5 fill:#5f3a1e,stroke:#be8f4d,color:#fff
    style SHADOWS fill:#3a1e5f,stroke:#8f4dbe,color:#fff
    style GRID fill:#3a1e5f,stroke:#8f4dbe,color:#fff
    style EXTRACT fill:#1e3a5f,stroke:#4d8fbe,color:#fff
```

Two of those run **before** anything is drawn, and the order is not
arbitrary: shading samples the shadow atlas and reads the froxel grid, so
both have to be filled first. They are separately scoped because a shadow
pass that costs four culls and four rasters was, until #785, hiding
inside whatever number the frame reported.

The R64 path's `raster + shade` is **one fused scope** covering
everything from the clear to the sharpening. Its children — motion
vectors, the shade dispatch, the upsample, the temporal resolve, the
tonemap, RCAS — are timed individually inside it, which is how a capture
answers *which half of the fused pass is the cost*.

### Cull

Two levels. **Instances first**, one thread each, testing the mesh's
bounding sphere against the frustum and then against a screen-size
threshold. Every survivor reserves `⌈its own meshlet_count / 64⌉`
*chunks* in a list, and a single thread turns that count into indirect
dispatch args. **Then meshlets**, one workgroup per chunk, one lane per
meshlet — so the meshlet domain is entered at each instance's own count.

Each meshlet thread tests its meshlet and, if it survives, appends its
`(instance_id, meshlet_id)` to a `visible_meshlets` buffer with an
atomic bump. The draw that follows is `draw_indirect` off a count the
GPU wrote — the CPU never learns how many meshlets survived, and does
not need to. Neither does it learn the chunk count: the expansion is
`dispatch_workgroups_indirect` off a number that exists only on the GPU,
because a readback in the hot path is a frame of latency.

Tests, in order: **frustum** against the meshlet's AABB, **backface** via
its normal cone, and **LOD chain descent** — a meshlet is drawn when its
own screen-projected error falls under the target and its parent's does
not. The instance level runs the two passes of the LOD descent over the
same chunks, because a group that descends in one pass and not the other
is a hole in a surface.

> 🔴 This was one dispatch, and its shape was a RECTANGLE:
> `instance_count × the heaviest mesh registered anywhere in the scene`.
> Every thread past a mesh's own meshlet count existed to fail a bounds
> check. On `dense.scene` that was **9 633 630 threads for about 116 000
> real meshlets — 98.8 % padding**, and the frame was CPU-bound on it.
>
> The property that made it a scaling bug rather than a constant factor
> is that it was **contagious**: importing one detailed prop raised the
> stride for every instance of every other mesh, so a field of
> one-meshlet cubes got slower because a dragon existed. What remains is
> at most 63 wasted lanes per surviving instance — a constant, and one
> nothing else in the scene can change.

**Render distance** hangs off the instance level, as
`meshlet_min_pixels` — an instance whose bounding sphere projects to
fewer than that many pixels is rejected before it becomes meshlets.
`0` is off, which is what ships.

> 🔴 A size, not a distance, and that is forced rather than chosen. The
> projection is `perspective_infinite_reverse_rh`; `far` never arrives
> and `ndc.z = near / distance` being exact is what contact shadows,
> SSR, fog, the atmosphere and the temporal upscaler all read. A far
> plane would break five consumers to serve one.
>
> It reaches `cs_cull_instances` and nothing else — deliberately. A
> shadow cascade is orthographic and its "pixels" are shadow texels;
> rejecting a caster because it is small on the CAMERA is how a shadow
> loses the object throwing it.

> 🔴 The LOD selector read the projection scale from a single matrix
> element for a long time. That element is `f × (camera up · world up)`,
> so it is correct for a level camera, smaller for a tilted one, and
> **zero at 90° of roll or looking straight down** — which switched the
> selector off entirely. It now takes the norm of the row that produces
> `clip.y`. Any non-level view had been losing detail since continuous
> LOD shipped, degrading smoothly enough to read as "that is how the
> model looks".

### Visibility buffer

Instead of shading during rasterisation, the raster pass writes only
*which triangle covered this pixel*. Shading happens afterwards, once per
pixel, for the triangle that won.

**R64 path.** The fragment shader does one
`textureAtomicMax((depth << 32) | ids)` into an `R64Uint` storage
texture. Depth in the high bits means the atomic max resolves depth and
identity in a single operation — no depth buffer, no z-fighting between
coplanar meshlets, no ordering.

**R32 path.** Without 64-bit atomics the same idea runs in two passes
against a Hi-Z pyramid built with single-pass-downsample: pass A draws
what was visible last frame, the pyramid is rebuilt from that depth, and
pass B recovers whatever pass A wrongly occluded. Metal has no
`atomic_uint64`, so this path is not legacy — it is the Apple path.

### Shading

Both paths reconstruct the surface the same way, through
`surface_reconstruct.wgsl`: perspective-correct barycentrics from the
triangle's three world-space positions, giving world position, normal,
uv, tangent and **analytical uv derivatives** — the automatic ones are
wrong here, because neighbouring fragments in a 2×2 quad may come from
different triangles.

Only the visibility-buffer *read* differs between the paths. That was not
true until #441: the R32 path averaged the triangle's three vertex
normals and never computed a world position at all, which was invisible
while shading was a function of the normal alone and would have lit the
centroid of every triangle the moment a point light needed a distance.

- **R64, fragment — the default.** One fullscreen pass *per material*,
  depth-testing `Equal` against a target holding each pixel's material
  id. The depth test is the per-material cull, in hardware, with
  early-Z. Each pass binds its own textures.
- **R64, compute — opt-in.** One dispatch for the whole screen, into an
  HDR target. Turned on per project (`compute_shading`) or per run
  (`KOOCH_COMPUTE_SHADING`), and it is what half-rate shading and the
  reduced-rate upsample require.
- **R32** shades with one compute dispatch. **No texture sampling**: a
  compute shader has no implicit derivatives, and `textureSampleGrad` is
  a fragment-stage call. Scalars only.

> 🔴 **"Per material" means per material in the PROJECT, not in the
> frame.** `MaterialPipeline::shading_slots` is `0..next_slot` and
> `sync_from_resources` registers every `Material` the `AssetDatabase`
> knows about, so dropping an unused `.ron` into the project's folder
> adds a full-screen sweep to every frame. A tile that owns none of a
> slot's pixels does no reconstruction and writes nothing, but it does
> not leave for free: every thread still reads the R64 vbuf, chases
> `visible_meshlets` and then `instances` off that read, and waits on
> three unconditional barriers.
>
> `KOOCH_SHADING_PAD=<n>` appends `n` sweeps whose `material_id` matches
> no instance. The frame is bit-identical — every store in
> `material_frame_compute.wgsl` is inside the branch that never fires — so
> the only thing an A/B across it measures is what an idle sweep costs.
>
> **Measured on the OneXFly at 1920x1080, 2026-08-20: 178 µs a sweep**
> (1.98 µs on a desktop 9070 XT at 1280x720). A sweep is a fixed dispatch
> cost plus per-pixel work, so quote the resolution with the number. `roll-a-ball` has three materials and pays 0.71 ms a
> frame — 22 % of its own shading pass. A game with twenty pays 3.7 ms.
> Use a pad in the hundreds when measuring: four extra sweeps sit under
> the device's own run-to-run drift.

> 🔴 **A `serde` default is not a recommendation — it is what an old file
> silently becomes.** `compute_shading`, `shading_rate` and `temporal_aa`
> all default to what the engine already did — fragment path, full rate,
> no history — because an earlier version defaulted two of them to *on*
> and every existing project changed shading path and gained a temporal
> resolve in the same build. Two variables at once is not a change
> anybody can bisect, and the first report was "you broke the whole
> render".

### The window, and everything being live

`window_mode` sits beside `vsync` in the Presentation group: **0
windowed, 1 borderless, 2 fullscreen, 3 exclusive**.

| mode | what it is | changes the output resolution? |
|---|---|---|
| Windowed | a decorated window at the project's size | no |
| Borderless | the same size, no title bar — still a **window** | no |
| Fullscreen | the monitor, at the monitor's **current** mode | no |
| Exclusive | asks the display to **change mode** | **yes** |

🔴 **Exclusive does not work everywhere, and the engine says so rather
than letting it fail quietly.** Windows and X11 implement it; winit
ignores it on Wayland and its own source says so twice —
`warn!("`Fullscreen::Exclusive` is ignored on Wayland")`, which leaves
the window exactly as it was and reads as the setting being broken. So
`window_mode::effective` degrades the request to fullscreen before it
reaches winit, with a warning naming both modes.

### The resolution, and what the monitor reports

Two resources carry it, both live:

- **`Resolution { width, height, refresh_mhz }`** — what the game asks
  for. In windowed and borderless it is the window's inner size, applied
  through `request_inner_size`; in exclusive it is the display mode to
  switch to. `refresh_mhz: 0` means *"the best this size can do"*.
- **`DisplayModes { modes, exclusive }`** — what the platform will
  actually do, published once the window exists. The list comes from the
  **player's** monitor rather than from a constant, and `exclusive` is
  false under Wayland. A game's options menu is built from this: a
  resolution dropdown that changes nothing is worse than none.

🔴 **In exclusive, the size has to match a mode exactly.** Substituting a
nearby resolution would change what the player sees without saying so,
so the fallback is borderless fullscreen at the monitor's own size, with
a warning. Among modes of the right size the engine takes the one closest
to the refresh asked for, or the highest when none was.

⚠️ `request_inner_size` is a **request**. Wayland answers with a
configure event rather than a return value, which the engine already
handles: `WindowResized` → `GpuContext::resize` → the render targets.

⚠️ The **environment override** `KOOCH_WINDOW_MODE=windowed|borderless|
fullscreen|exclusive` is applied when the window is created; the
**asset's** value lands a few frames later, because the settings asset
needs the asset server, which needs the GPU, which needs the window.

⚠️ None of this reaches a handheld under gamescope. The compositor hands
the game a surface at the resolution **it** was configured for and scales
the result; the knob that decides there is outside the process.

**Every setting in the file is live**, which is what makes a game's own
options menu possible:

| setting | how it lands |
|---|---|
| exposure, ambient, shadows, contact shadows | resources read per frame |
| `compute_shading`, `shading_rate`, `upscale`, `sharpening` | applied per frame on the stage |
| `render_scale` | next frame — `render_frame_system` calls `resize` once a frame and it early-returns unless the size or the scale moved |
| `anisotropy` | rebuilds one sampler when the number changes |
| `vsync` | reconfigures the surface when the mode differs |
| `window_mode` | `set_fullscreen` / `set_decorations` when the window is not already like that |

Each of those is guarded on "is it already what was asked for", because
every one of them is a reallocation of something — a swapchain, a
sampler, a set of render targets — and applying it unconditionally would
rebuild it once a frame.

### 🟢 What a handheld ships with

Measured on the OneXFly at 10 W, settled, 1920x1080, 2026-08-20:
**13.92 ms median, GPU 9.7 ms**, against a 13.9 ms budget.

```ron
compute_shading: true,   // the tiled path; half rate needs it
shading_rate: 2,         // Half — one sample per 2x2 quad
upscale: 2,              // SGSR 2
render_scale: 50,        // Performance — 50 % (2x)
```

🔴 **Cap the frame rate, and not only for the battery.** At 1280x720 the
same frame costs **3.9 ms of GPU capped at 72 fps and 13.2 ms uncapped**
on this part. Capped, the GPU is idle 68 % of the time, so it never
reaches its power cap and holds ~1210 MHz; uncapped it throttles and
every pass takes three times longer. Rendering 144 frames to display 72
pays for the same work three times — and the cap fixes the pacing too:
max frame 15.25 ms against 47.09.

⚠️ The capped run was also at a higher TDP than the uncapped one, so the
3.4× is the cap and the power together. `gpu_busy_percent` reading 32 %
argues the cap is most of it — a part idle two thirds of the time is not
power-limited — but the run that separates them has not been taken.

🔴 **The upscaler is the largest single choice on that list.** Same
scene, same build, same session: `upscale: 3` (FSR 3.1) costs **11.355
ms** and `upscale: 2` (SGSR 2) costs **2.062** — a 23.36 ms frame against
a 13.92 ms one. FSR 3.1 is not broken; it is a desktop technique, and its
own dropdown entry says so.

### DLSS (#536)

`upscale: 4` is NVIDIA's, and it is the only technique here a build can
**lack**. The other three are shaders this engine owns; DLSS is a neural
network shipped as a binary blob, reached by linking NVIDIA's SDK.

Three things follow, and all three are visible from a project:

1. 🔴 **It is a compile-time feature.** `dlss_wgpu`'s build script links
   `libnvsdk_ngx` statically and runs bindgen over NVIDIA's headers, so
   a binary either linked it or did not. Turn it on by adding `dlss` to
   a build preset's **Extra cargo features**; the editor supplies
   `DLSS_SDK` and `VULKAN_SDK` and refuses to start cargo when either
   the SDK or the **Vulkan headers** are missing. The headers are a
   separate package from the loader — a machine that runs Vulkan games
   has no reason to carry them — so the editor's startup check lists
   them alongside Rust and ALSA, in the same one-paste command.

   ⚠️ What cargo actually receives is **`kooch/dlss`** — `dlss` on its
   own names a feature of *your* crate, which you never declared, and
   cargo answers that with *"the package does not contain this
   feature"*. The editor rewrites the bare spelling for you, unless your
   own `Cargo.toml` declares a `dlss` feature, in which case it means
   yours and is left alone.
2. 🔴 **It moves the whole build to Vulkan.** `dlss_wgpu` is Vulkan-only,
   and on Windows wgpu picks D3D12 by default. Enabling DLSS therefore
   moves *every* Windows player onto Vulkan, not only the ones with an
   NVIDIA card. That is a decision about the whole build, which is why it
   is a feature rather than a setting.
3. 🟢 **Asking for it is always safe.** A build without the feature, or a
   machine without an NVIDIA card, resolves with the engine's own TAA at
   full resolution and says so once in the log. The `.rendersettings`
   file is left alone: `upscale: 4` is what the project wants, and the
   machine that can honour it should still see it.

**Getting the SDK.** Settings → the DLSS button clones
[NVIDIA/DLSS](https://github.com/NVIDIA/DLSS) at the pinned tag after you
accept NVIDIA's terms, into `~/.local/share/kooch/sdk/dlss/<version>/`.
The engine never mirrors it — hosting a copy is the "stand-alone
product" the licence forbids.

**Shipping it.** A build with the feature gains two files beside the
executable, copied by the packager rather than by you:

| file | why |
|---|---|
| `libnvidia-ngx-dlss.so.<ver>` / `nvngx_dlss.dll` | NGX `dlopen`s it from the application's own directory. Nothing links it, so there is no `rpath` to get right |
| `DLSS_NOTICES.pdf` | Section 9.5 of NVIDIA's Programming Guide, which anyone distributing the blob must include. Copied because a licence file nobody remembered is a licence breach |

⚠️ **Unmeasured.** DLSS has a number on no device in this repository yet.
It is a desktop option with an NVIDIA card; the handheld's default stays
SGSR 2 at **2.062 ms**, and a vendor backend that does not beat ours by a
number does not get to be a default.

**Dropping the output resolution buys more than any of these**, because
`render_scale` is a percentage *of the output*: a smaller window shrinks
the render target with it and everything `render_scale` does not touch —
the resolve's output, the tonemap, the blit.

⚠️ These are a recommendation, not defaults. `RenderSettings::default()`
stays what the engine did before any of them existed, for the reason the
callout above gives: a serde default is what an old file silently
becomes.

Then [Inti](./lighting.md) — Cook-Torrance driven by the scene's lights.

### Layers (#1218)

Every `MeshRenderer` carries a 32-bit `layers` mask, and the scene walk copies it into
`MeshInstance.layers` — so the mask is on the GPU, beside the flags, for whatever filters by it to
read without asking the ECS again. What each bit is called lives in the project's `.layers` file,
which colliders name their groups from as well: one table, so a bit means the same thing wherever
it is ticked.

A camera filters by it: `PerspectiveCamera.culling_mask` rides into `CullParams`, and `skipped()` —
the one place every cull path already asked whether an instance is this view's — rejects what shares
no bit with it. In the cull, so nothing walks the instances on the CPU, and per view, so the
editor's View panel keeps every layer while the Game panel keeps what the game's camera says.

🔴 **Shadows are not filtered by it.** A shadow view builds its own `CullParams`, whose mask is every
layer: what a camera does not draw still casts, because the shadow belongs to the light. A caster
hidden from one camera and lit by a lamp both cameras see would otherwise lose its shadow in both.

A light filters by it twice, because they are two different wishes:

- **`layers` — what it lights.** The test is per pixel, at the top of `inti_light_lit`, where the
  instance's layers ride in `IntiSurface` beside its flags: one AND before anything is sampled, and
  the froxel lists stay exactly as they were. A lamp that lights the character and not the floor is
  this mask.
- **`shadow_layers` — what casts into it.** Rejected in that light's own cull, so a caster the light
  ignores is never rasterised into its map at all: `with_culling_mask` on the cascades, the spot
  maps and the cube faces, and the same test inside `lamp_cull.wgsl` for the virtual pages, which
  pick their casters on the GPU and have no CPU pass to filter them in.

### Camera stacking (#1221)

A frame is a **list** of cameras, not one. The base — the highest-priority active camera that is not
an overlay — owns the image: its sky, its clear, its post-process. Every camera with `overlay = true`
is drawn after it, lowest `priority` first, and the last one is on top.

Each camera of the stack is a **view of its own**: its own colour and depth attachments, its own
culling mask, its own lens, its own temporal history — which is why an overlay gets a view that lives
across frames rather than a fresh one per frame, or SGSR2 would never converge. `StackViews` keys
them by camera entity and frees the view when the camera goes.

Composing is the blit that was already there. The stage leaves alpha at 0 wherever nothing was drawn,
so an overlay blended over the base keeps the base everywhere it drew nothing — no second sky, no
clear, no depth test between the two. 🔴 The blit **discards** those empty pixels rather than writing
them: the blend hides a wrong composite in the colour, but its depth would be wiped, and the grid,
the gizmos and the transparents drawn afterwards test against that depth.

🔴 **Alpha is coverage, not opacity**, and every pass that rewrites colour has to carry it: the
tonemap, TAA and RCAS already did, SGSR 2 wrote 1.0 and an upscaled overlay composed as an opaque
black plate with its own objects on it — the base gone. It now takes the coverage from *this* frame's
input, never from the history, which would ghost a hole into the image.

An overlay with no base composes nothing. There is nothing under it to keep, so it would read as the
whole image, which is not what it asked for.

A virtual camera drives the camera carrying a `CameraBrain`, and only that one. "The highest-priority
camera" was the rule while a frame was one camera; with a stack it hands the rig to whatever overlay
outranks the base. A scene where no camera carries an enabled brain is driven by nothing and says so
once in the log — a rig that moves a camera nobody pointed it at is the bug, not the fallback. An
overlay that wants its own rig, a weapon camera, carries its own brain.

What it costs: one scene pass per camera. An overlay is a second cull, a second raster and a second
shade of whatever its mask keeps — a UI layer of a handful of instances is cheap, a second full scene
is not.

### Transparent surfaces (#452)

Two kinds blend: `transparent`, a lit surface with an `alpha`, and `transparent_unlit`, the same
without a single light — fire, energy, a sprite that carries its own brightness. They take the same
path everywhere; what the kind decides is one const-folded branch in `transparent_lit.wgsl`, where
`SURFACE_UNLIT` skips Inti. `ShaderKind::blends()` is what every router asks, so a third blended
kind would be one variant and no new `if`.

A blended material's instances are appended after every opaque one. The view's cull is
handed the opaque count, so they never reach the visibility buffer; the shadow culls — cascades and
pages — are handed every instance, so they cast. A renderer with `cast_shadows` off carries
`INSTANCE_CASTS_NO_SHADOW`, and every shadow view's cull (`CullParams::shadow`, and the lamp cull)
skips it.

Their shadow follows their alpha (#1224) without any shadow raster knowing about materials: each
frame, before the shadows, `ShadowAlpha` bakes every transparent caster's coverage over its uv
square into one layer of a 128×128 `R8Unorm` array (32 layers), with a table naming each material
slot's layer. The shadow rasters — cascades, spot and point faces, the virtual pages — switch to a
variant whose fragment samples that coverage at the corner's uv and drops the fragment against a
4×4 Bayer threshold, which the shadow filter averages into partial shadow. Only while a
transparent material casts: otherwise the classic rasters keep their fragment-less pipeline. A
caster whose shader reads `input.time` hashes the frame into its instance hash, so its cached
pages and cube faces redraw instead of freezing.

They are drawn on the compute path after the shade (and its upsample) and before the temporal
resolve, from a packed `(instance, meshlet)` list of each instance's finest meshlets:

1. **Insert** — one raster of both faces for every material. Each fragment builds a 64-bit key,
   depth above and `(slot, triangle)` below, and offers it to its pixel's four layers with
   `atomicMax`, carrying the smaller down; what leaves the last layer raises an overflow flag. The
   opaque depth is tested in the shader: a fragment that writes storage runs before a late depth
   test would reject it.
2. **Tail** — a compute zeroes the tail's indirect draws when nothing overflowed. Otherwise each
   material rasterises again and keeps only fragments behind the fourth layer, into McGuire and
   Bavoil's weighted blended targets (Hybrid Transparency, Maule et al. 2013).
3. **Shade** — one compute per material lights the layers whose key names it, with the same
   `resolve_surface` a visibility-buffer sample uses, and packs colour and coverage back into the
   layer as four halves.
4. **Composite** — the layers front to back, then the tail, blended premultiplied over the radiance.

The layers are four `u64` a pixel — 33 MB at 1280×800 — allocated at the first frame with a
transparent material. The layers need `SHADER_INT64_ATOMIC_ALL_OPS`; without it,
or when they would not fit one storage binding, a sorted pass draws instead: instances far to near,
back faces culled, blended as they land.

The tail's per-material draws start past instance 0, which wgpu's indirect validation drops unless
`INDIRECT_FIRST_INSTANCE` is enabled; the engine requests it where the adapter has it, and without
it draws each run directly.

### Masked surfaces (#452)

A shader that assigns `alpha_clip` is masked, and each masked material rasterises in a **bin** of its
own — Nanite's programmable raster, for the same reason: the opaque raster is one material-less
draw, and a cut needs the material.

- **Assign** (CPU, per frame, before the instance upload) — `MaskedRaster::assign` gives each
  masked material whose pipeline compiled a bin, up to 32, writes a material-slot → bin table and
  sets `INSTANCE_MASKED` on its instances. The opaque rasters (R64 and R32) skip flagged instances
  in the vertex stage. A material that got no bin is never flagged, so it draws solid rather than
  vanishing.
- **Bin** (after every cull, the R32 path's two included) — `masked_bins.wgsl` counts the visible
  masked meshlets per bin, takes the offsets and indirect args in one thread, and scatters their
  visible-list slots bin after bin. Strided, so no dispatch passes 65 535 groups.
- **Draw** (inside the raster pass, after the opaque draw) — one indirect draw per bin, with the
  bin's material and textures. The fragment rebuilds the pixel with `resolve_surface`, exactly as
  shading will, runs the material's `surface` and discards below `alpha_clip`; a survivor writes
  the same `(slot, triangle)` id the opaque raster would. Shading never learns a pixel was masked.

Shadows reuse the transparent bake: a masked material's layer holds its cut (`alpha >= alpha_clip`)
and carries a table bit that makes the shadow rasters read it against 0.5 instead of the dither.

A transparent shader that assigns `alpha_clip` gets an insert pipeline of its own
(`TRANSPARENT_CLIP_INSERT_FRAME`), drawn run by run beside the shared one: its fragment runs the
surface and drops what falls below the clip before `atomicMax`, so a cut fragment never holds a layer.
`transparent_lit` answers a coverage of -1 there as well, which the tail and the sorted fallback
discard.

### Coverage hulls (#452)

A cut that cannot move can come out of the mesh instead of out of every pixel — not exactly, but
generously. `Shader::masks_still` answers whether the source mentions `input.time`,
`input.world_position`, `input.camera_position` or `input.frag_coord`; without them the coverage is
a function of uv and textures alone, and `AlphaTrim` shrinks the mesh to a **hull** around it, once
per (mesh, material, values):

- **Settle** — the pair has to ask with the same `MaterialPipeline::slot_stamp` for 8 frames, so a
  dragged slider never bakes. One pair a frame, in `sync_assets_to_gpu`, before the generated drain
  that uploads what it publishes.
- **Bake** — the shadow bake's own frame (`shadow_alpha_bake.wgsl`, its square now a uniform) over
  256², read back to the CPU. 🔴 The readback blocks: it belongs to the asset step, and 64 KiB once
  per pair is what it costs. A masked material bakes its cut, 0 or 1; a transparent one bakes its
  alpha, and anything above zero counts as covered, because it still blends there.
- **Hull** — the mask is grown by a margin, `contour` walks marching squares over the grown mask,
  and `geo` simplifies by no more than that same margin. Douglas-Peucker can only cut back into what
  the margin added, so **the hull always contains the coverage**. Too many corners for the budget
  (16): grow the margin and walk it again, up to 16 texels. A disc takes about ten triangles.
- **Cut** — each LOD 0 triangle is classified against the **grown** mask by its uv bounds: all kept,
  all gone, or clipped against the hull with `geo`'s boolean ops and triangulated by earcut. New
  corners interpolate position and normal across the source triangle; pieces are rewound to the
  triangle they came from, because earcut hands back its own winding and the raster culls back
  faces. Bit-equal vertices weld, then `build_meshlets_lod_chain` rebuilds the chain.
- **Budget** — past 32 triangles, or keeping more than 90% of the mesh's uv, the cut is refused: the
  vertices would cost more than the fill they save.
- **Draw** — the cut mesh is published as a `GeneratedMeshes` entry and the scene walk swaps it in
  for that (mesh, material) pair. Nothing else changes: the material still runs its masked raster or
  its transparent insert **inside** the hull, and the shadows still read the coverage bake. What the
  hull saves is the fill of everything the alpha never reached — the empty corners of a leaf card,
  the space around a sprite, and for a transparent one the fragments that never enter the layers.

Prior art: Humus' particle trimming and Unity's tight sprite mesh, both of which enclose the sprite
in a handful of corners rather than following its edge. Tracing the alpha exactly buys the same fill
for a mesh nobody wants to pay for.

A pair that cannot be cut — a tiled uv, a mesh generated rather than loaded, a coverage that fills
its own square — is remembered as refused and keeps the mesh it was authored with.

### After the shade: rate, history, and the tonemap

Three passes sit between Inti and the sky, and all three exist on the R64
path only.

- **Half-rate shading.** `KOOCH_SHADING_RATE=half` shades a quarter of
  the pixels and `shade: upsample` puts them back on screen. The scope
  renames itself — `shade: compute (half rate)` — so a capture answers
  *which rate produced this* without anyone trusting a log line. The
  cheap half of the frame stays full-rate: the visibility buffer, the
  depth, the motion vectors.
- **`motion vectors`.** Each pixel's previous clip position, from the
  camera's *unjittered* matrix. The jittered one goes to everything else
  — cull, Hi-Z, raster, every reconstruction that reads the buffer the
  raster wrote — and the pair being separable is the whole reason the
  vectors are not wrong by a sub-pixel offset every frame.
- **`taa`, and it is off by default.** The resolve exists and works;
  turning it on is #481's remaining half. Debug views bypass it, because
  averaging a false-colour legend across frames is not a legend any more.
- **`rcas`, and it is off by default.** Robust Contrast Adaptive
  Sharpening, one full-screen pass, `sharpening` in `.rendersettings`.
  🔴 It runs **after** the tonemap, unlike everything else in this list:
  RCAS is adaptive because it solves for the filter weight at which the
  signal would clip out of `{0, 1}`, and handed radiance in the hundreds
  that limiter stops limiting. When it runs, the tonemap resolves into
  its texture instead of into the window. Reconstruction is soft by
  construction — a resolve builds each output pixel from samples that
  landed *near* it — so at a `render_scale` below 100 this is not polish.
- **`tonemap`.** Shading writes **HDR radiance** into a linear target and
  the tonemap converts it at the end, because TAA has to run on linear
  radiance. The operator is *concatenated* from Inti rather than
  reimplemented — two copies of a curve that must agree to within one
  255th is how a parity test starts failing for a reason nobody can find.

> 🔴 Everything that compresses range — the resolve, the tonemap, any
> firefly clamp — needs the **exposure applied first**. Radiance in this
> engine is in the hundreds, and `c / (max(c) + 1)` on those numbers
> posterises into flat bands that read as a broken toon shader rather
> than as a missing divide.

### Sky and composite

The sky is a fullscreen pass: procedural gradient plus volumetric clouds
(3D value noise FBM, Beer–Lambert transmittance, Henyey–Greenstein
phase, in-scattering toward the sun). It draws first, and the meshlet
stage's colour is blitted over it — `alpha = 0` is the background
sentinel, so pixels no meshlet covered keep the sky.

> ⚠️ `GpuContext` deliberately selects a **non-sRGB** surface format, on
> the reasoning that "most renderers handle gamma correction in the
> shader". Inti does. **The sky pass does not.** If the two disagree on
> brightness, that is the sky's half of a decision taken long ago and
> never finished.

## Debug views

`MeshletDebugMode` is a `Resource` the editor sets per frame; the shaders
branch on a single `u32`. `Off` is the production path.

| Mode | Shows |
|---|---|
| `MeshletIds` / `InstanceIds` | Cluster boundaries; per-entity coverage |
| `TriangleDensity` | Triangles drawn per pixel — calibrates `target_error_pixels`. Anything brighter than green is sub-pixel triangle territory |
| `Overdraw` | Visibility-buffer atomic writes per pixel |
| `FrustumRejected` / `BackfaceRejected` / `HiZRejected` | What each cull stage discarded |
| `CullPassthrough` | Everything that survived every stage |
| `OnlyLod0` / `OnlyRoots` | The two extremes of the LOD chain, in isolation |
| `Normals` | The world-space normal as colour |
| `ShadowCascades` / `ContactShadows` | What each shadow mechanism saw — see [Inti](./lighting.md) |
| `SingleLight` | The selected light, alone, in grey, with its shadow |
| `LightsPerPixel` | How many lights the pixel actually evaluated. Cost becomes a property of *where the pixel is*, which no pass timing can show — `raster + shade` is one number for the whole screen. 🔴 A flat maximum means the frame is **not clustering**: every light, every pixel |
| `PointShadowFactor` | One point light's cube map answering for itself — no BRDF, no cosine, no exposure, no second light. Magenta: no casting lamp. Blue: past its `range`. Grey ramp: the factor |
| `PointCubeFaces` | The cube map itself, six faces in a 3×2 grid (+X, −X, +Y, −Y, +Z, −Z). Dark blue is *nothing recorded*, which is what an occluder culled out of the map looks like |
| `WireframeOver` | The same edges over the shaded frame, so the mesh is read against what it is drawing. Drawn after the tonemap on the R64 path and inside the shade on the R32 one, and the only debug view that keeps the production frame underneath |
| `Wireframe` | Every triangle's edges, over a dark plate. A pixel whose right or lower neighbour carries another `(slot, triangle)` is an edge, so it costs two taps on the visibility buffer and no reconstruction. What a LOD or a trimmed mesh (#452) actually rasterises, in the only unit that matters: pixels of line |

The last three exist because *"the shadow is not there"* is four faults
wearing one pixel — no lamp near this point casts, the point is past the
lamp's reach, the cube says lit because the occluder never reached the
map, or the cube says dark and the other lamps fill it back in. Four
fixes, one colour in a shaded frame.

`Normals` deserves a note: until #441 it *was* the shading model. The
renderer computed `normal * 0.5 + 0.5` and multiplied by albedo, which is
why a scene with lights and a scene without them rendered identically.
It survives as a debug view because it is a genuinely useful look at the
geometry — it just stopped being what you get by default.

The colorize views (the ids, the heatmaps, the passthrough, the wireframe) draw
over the window while reading a render-sized visibility buffer, so the pass
stretches between the two sizes. Reading one to one left them in a corner
whenever the render scale was under 100%.

The atomic-counter modes need `TEXTURE_ATOMIC`; the editor's dropdown
hides what the adapter cannot run rather than offering a mode that
silently falls back.

The Inti-side views — `Normals`, `ShadowCascades`, `ContactShadows`,
`SingleLight` — are **not compiled into the shader a game runs**. They
live in `inti_debug.wgsl`, which only the editor's second pipeline
concatenates; production takes `INTI_DEBUG_STUB` instead and the call
sites fold to `if (false)`. The reasoning, and why an untaken branch is
not free, is in [Inti](./lighting.md#the-debug-views-are-not-in-the-shader-your-game-runs).

## Depth: reversed-Z, and no far plane

The camera's projection is `perspective_infinite_rh_reverse_z`. Near maps
to `ndc.z = 1`; infinity approaches `0` without reaching it. Depth
attachments clear to `0.0` and compare `Greater`.

The property worth knowing, because half the renderer leans on it:

```text
ndc.z == near / distance
```

Exactly. Any shader recovers metres from the depth buffer with one
divide and no extra uniform — which is why the contact-shadow march can
take `thickness` and `length` in world units and have them mean the same
thing in every scene. With a finite far plane it takes two coefficients
plumbed to every consumer, and the first one that forgets ships a
parameter documented in metres that does not measure metres.

Two things follow, and both are load-bearing:

- **The far plane is gone from culling too.** That row of the projection
  degenerates to a zero-length normal; `extract_frustum_planes` returns
  `[0,0,0,0]` for it and the cull shader walks five planes.
- **Unprojecting uses the NEAR plane.** `ndc.z = 0` is infinity now and
  unprojects to `w = 0`. Anything that builds a ray from a cursor takes
  `ndc.z = 1` — same ray through the eye, always finite.

The bounded `perspective_rh_reverse_z` survives for shadow cascades: a
slice of an unbounded frustum is unbounded. Rationale and the full list
of what this touched: [ADR 0002](../../../decisions/0002_infinite_reverse_z.md).

## Limits worth knowing

- 🔴 **65 536 instances.** The visibility buffer packs
  `(instance_id << 16) | meshlet_id`. A chunk of vegetation exhausts
  this. Bevy removed their equivalent limit in 0.17 with BVH culling.
- 🔴 **Six bind groups, six used.** The two-pass shading pipeline uses
  every group `TARGET_MAX_BIND_GROUPS` allows. Shadow maps have to go
  *inside* Inti's group — which is where they belong anyway, since a
  shadow map without its light is not a thing any shader wants. Raising
  the target to 8 would work on desktop and drop a baseline Vulkan only
  guarantees at 4.
- **Skinned meshes cull against their bind pose**
  ([#453](https://github.com/lobinuxsoft/kooch/issues/453)), so an
  animation that reaches outside the rest volume culls a character who
  is on screen.
- **The R32 path has no motion vectors and no history**, so no TAA and no
  temporal upscaling there. Jitter on that path is a wobble and nothing
  else, which is why it is not applied.
- **TAA ships off** ([#481](https://github.com/lobinuxsoft/kooch/issues/481)).
  The resolve is built and the vectors feed it; what is missing is the
  half that turns it on by default without softening a still image.

## Not in the pipeline yet

Shadows, contact shadows and clustered shading used to be listed here.
Cascades ([#476](https://github.com/lobinuxsoft/kooch/issues/476)) and
contact shadows ([#735](https://github.com/lobinuxsoft/kooch/issues/735))
shipped, and the froxel grid
([#780](https://github.com/lobinuxsoft/kooch/issues/780)) runs every
frame — the passes in the diagram above are what replaced those bullets.
What is genuinely still absent:

- **Virtual shadow maps** ([#477](https://github.com/lobinuxsoft/kooch/issues/477)).
  Cascades cover the sun; a hundred shadowed point lights each want a
  cube map, and that is the wall VSM exists to move.
- **Global illumination** ([#450](https://github.com/lobinuxsoft/kooch/issues/450)) —
  surfel + voxel, not raytraced. Its absence is why punctual light
  defaults are larger than physics says they should be; see
  [Lighting](./lighting.md).
- **Atmosphere** ([#250](https://github.com/lobinuxsoft/kooch/issues/250),
  [#248](https://github.com/lobinuxsoft/kooch/issues/248)) — correct from
  orbit, and tinting the sunlight.
- **The post-processing stack**
  ([#254](https://github.com/lobinuxsoft/kooch/issues/254)) — AgX, SMAA,
  vignette. The `tonemap` and `rcas` passes exist; the stack around them
  does not, and exposure is a setting rather than an auto-exposure loop.

## Why there is no render graph

There *was* one — `kooch_render::graph`, 406 lines, cycle detection and
topological sort — and **nothing ever instantiated it**. The real
renderer was built beside it, and the module is gone (#392).

The decision not to revive it is not laziness. Bevy 0.19 **deleted their
`RenderGraph`** and replaced it with ECS schedules, because the graph ran
as an exclusive system and was single-threaded — the engine that made
the pattern canonical retired it. Kóoch already has the replacement half
written: `kooch_core`'s scheduler batches GPU systems into a shared
encoder — and a `GpuSystem` **records into that encoder**, opening the
passes it needs (a compute pass, a render pass into a target, or
several), wrapped in a debug group carrying its name. A system also says
where it runs inside its stage:

```rust
app.add_cpu_ordered(Stage::Render, Order::after("render_frame_system"), MyPass);
```

Targets come from a pool: `TargetPool::acquire` hands back a target
matching a `TargetDesc` — reused when a free one matches, created
otherwise — and `release` makes it reusable at once: reusing a texture is
ordered by the queue like any other write. Two views of one size cost one
set of targets. A target no pass asked for in the last three frames is
dropped at `TargetPool::end_frame`, which whoever presents the frame calls
once: three frames is past the two in flight, so Mesa radv never sees a
texture dropped while the GPU still reads it, and dragging a panel edge
through a hundred sizes keeps the targets of the last few rather than a
hundred.
A post-process asks the pool for somewhere to draw rather than owning a
texture of its own.

The scene's `PostProcess` stack runs through one function,
`post_process::run_stack`, called by the editor's viewports and by the
game window. A swapchain image cannot be sampled, so when a stack is
active the game window draws the sky and the blit into a pooled target,
runs the stack over it, and copies the result onto the swapchain. The
surface is configured with `COPY_DST` wherever the platform offers it.
Without a stack, the frame goes straight to the swapchain as before.

### Volumes (#1222)

What the stack *is* can depend on where something stands. A `PostProcessVolume` contributes its own
effects while a body is inside it, and the scene's `PostProcess` is the layer underneath — the look
with nobody anywhere.

- **The shape is the collider**, which must be a sensor. It is already the engine's way of saying
  "this region", and its interaction groups already say who counts: a volume that only a player
  triggers is a collision group, not a second filter. Unity reads its volumes' colliders the same
  way; what neither engine uses is the *event* system for the blend.
- **The solver is the gate, not the answer.** `SensorOccupancy` holds who is inside which sensor —
  the frames between the arrival and the departure the solver reports, which is the "stay" nothing
  else provides — and re-measures how deep each body is once a frame. Only occupied volumes are
  measured at all; the rest cost a bool.
- **The depth is the blend.** Zero at the surface, all of the volume a `blend_distance` in,
  smoothstepped between. 🔴 It fades **inward**, where Unity's fades outward: the sensor is what says
  a body arrived, so the surface is the first place a weight can be asked for. Fading outward would
  need a second, wider shape nobody authored.
- **The fold is Unity's.** Volumes apply in `priority` order, each moving what the ones below left
  *towards* its own value by its own weight — so a volume can turn an effect **down**, which a max
  or an add could never do. An effect no volume mentions keeps what the scene gave it.
- Sphere, box and capsule are measured analytically. A hull or a trimesh has no cheap answer, so it
  reports "fully inside" the moment the solver says a body arrived: the shape still works, it just
  cuts instead of fading.

A plugin draws through the same machinery. `kooch_plugin_render` holds
the GPU half of the plugin API — a `RenderPass` with `init` and `record`,
the target pool behind a `Targets` trait, and `engine.add_pass(stage,
order, pass)`. It is a separate crate so `kooch_plugin_api` keeps costing
nothing to link: a plugin that only moves entities never compiles wgpu.
`examples/example_post_process` is the whole thing in one file, ordered
after the scene and before the present without editing either.

`Order::before` / `Order::after` name a system — the same short name the
Systems panel shows — because a plugin has no handle to a system the
engine registered. A name nothing answers to is dropped (the plugin that
owns it may not be loaded); a cycle is logged and the stage runs in
registration order. A plugin gets the same thing through
`Engine::add_ordered`
([#392](https://github.com/lobinuxsoft/kooch/issues/392)).
