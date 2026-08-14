# Roadmap

What is next and why, ordered by what blocks what. Issue bodies hold the detail; this is the
map.

Companion to [`MEMORY.md`](MEMORY.md), which records decisions already made. If the two
disagree, `MEMORY.md` wins on *decisions* and this file wins on *order*.

**There is exactly one "Next" heading.** Everything else is `Backlog` or `Done`. Three sections
called Next is how a roadmap stops being read.

Last updated 2026-08-12 — **the frame on the OneXFly is fill-rate bound**: 72 ms against a budget of 13.9, the GPU at 96 %, and the whole thing collapsing 5.2× when the internal resolution drops. The order is set by a performance budget, and that budget now has a verdict in it, not just a number.

---

## The constraint everything is now measured against

**72 FPS at 10 W TDP on the OneXFly F1 Pro.** The bar a game made with
this engine has to clear. It does not clear it today.

That is **13.9 ms per frame**, on a gfx1150 iGPU, on a third of the power
the part will draw if you let it.

| | |
|---|---|
| Whole frame | **13.9 ms** |
| A pass taking 2 ms | **14 % of the budget** |
| A pass taking 5 ms | **36 %, and nothing else has moved yet** |

This is not a section of the roadmap. It is the number every graphics
issue below now has to answer to, and it changes which of them are worth
doing. A feature that cannot fit in what is left of 13.9 ms is not a
feature this engine has.

⚠️ **At 10 W the GPU clocks well below its desktop behaviour.** A
measurement taken plugged in and unthrottled says nothing about this
target. Every number here is taken on the device, at 10 W.

---

## Next — the graphics queue, with the budget as the gate

🎯 **2026-08-13: the budget is reachable.** 13.89 ms at 1280×720 on the
OneXFly is 72 FPS — the number this whole queue was arranged around.
With clouds off, which is a condition and not a victory (see #731).

The order below is what the measurements say, not what the port list
said.

| | | Why here |
|---|---|---|
| ~~#743~~ | ~~light debug views~~ | **Done.** And they left the game's shader entirely — see below |
| ~~#777~~ | ~~spot light shadows~~ | **Done.** The shadow atlas became a `texture_depth_2d_array` on the way |
| ~~#776~~ | ~~`PointLight.radius`~~ | **Done.** Not 15 lines: six pieces, and `GpuLight` grew 64 → 80 B |
| ~~#778~~ | ~~point light shadows~~ | **Done**, feature and budget both |
| ~~#804~~ | ~~`receive_shadows` does nothing~~ | **Done.** The field existed and nothing read it |
| ~~#780~~ | ~~GPU clustering — the froxel grid~~ | **Done and measured** (2026-08-14). The busiest froxel holds 26 lights against 12 that reach a point — ~24 % over-listing, ordinary for clustering. It costs 0.15 ms. Not a suspect |
| **#824** | **shade in a compute pass, tile's lights in LDS** | 🎯 **Next.** `raster + shade` is **not ALU-bound** (#821) and the grid is not over-listing (#820), so what is left is 15 storage fetches per pixel. A tile can read them once. **Changes no pixel** |
| **#825** | shade at half rate, raster stays full | The frame falls **5.2×** with internal resolution — the strongest number measured. Possible only once shading is its own pass |
| **#826** | sample the tile's lights, 15 → 2-4 | The last axis, and the only one that changes the image. **Closes unbuilt if #824 + #825 meet the budget** |
| ~~#796 / #819~~ | ~~ReSTIR / Solari~~ | **Ruled out for this hardware.** Solari's world cache alone is 2.65 ms per refresh in Bistro on the author's machine — 19 % of our whole budget, on far faster silicon — and denoising runs through DLSS Ray Reconstruction, which the 890M has no path to |
| **#731** | volumetric clouds, froxel-based | The clouds are **off**, and that is the only reason the budget is met. They cost 39 ms as written |
| **#803** | 452 ms compiling pipelines on frame one | Load time, not frame time — but it is half a second of black screen every launch |
| **#254** | post + auto exposure | The blown-out white floor in three sessions of screenshots. Cheap |
| **#771 / #248** | atmosphere, ported from Bevy | Now worth doing for the sky it gives, not for what it saves: 1.2 ms without clouds |
| **#481 / #536** | motion vectors + FSR | Treats the symptom — fewer pixels of a pass that costs too much per pixel — but it is what the hardware is asking for |

### 🔴 What the frame is actually spending, 2026-08-13

Three captures, same scene, only the internal resolution changed. The
first controlled A/B of the whole investigation, and the only reason the
numbers below can be attributed to anything:

| resolution | frame (median) | p99 | `raster + shade` | `shadows` | `sky` |
|---|---|---|---|---|---|
| **1280×720** | **13.89 ms** ✅ | 15.22 | 6.89 | 0.72 | 0.15 |
| 1600×900 | 14.47 | 29.26 | 12.63 | 0.53 | 0.17 |
| 1920×1080 | 21.02 | 42.84 | 18.01 | 0.64 | 0.24 |

`raster + shade` scales **worse than linearly**: 2.25× the pixels costs
2.61× the time. `shadows` does not move with resolution at all — that
pass rasterises the maps, which is geometry and is cheap. **Sampling**
them happens inside `raster + shade`, per pixel and per light.

The cause, `inti_pbr.wgsl:1131`:

```wgsl
for (var i = 0u; i < inti.light_count; i = i + 1u) {
    radiance += inti_light_contribution(surf, inti_lights[i], frag_coord);
}
```

No per-pixel culling. A lamp across the map is evaluated in every
fragment, and if it casts, its shadow map is sampled there too. **The
cost is pixels × lights, multiplied** — which is why p99 at 1080p is
double the median, and why the user predicted it from the device before
the shader was read: *"anda mal cuando hay muchos objetos que reciben
luz y se muestran sombras"*.

The engine had already written this down, in `kooch_lighting/src/buffer.rs`:

> *"Inti shades every light for every pixel; past this count that loop
> is the frame. Clustering is the fix and is not implemented yet."*

That warning has since been rewritten — the fix is implemented, and what
the line now says is to check whether the grid is switched off.

### The clouds were 97 % of the sky

| | `cloud_coverage: 0.5` | `cloud_coverage: 0` |
|---|---|---|
| `sky` | **39.83 ms** | **1.22 ms** |

One scene field. The structural suspicion about that pass — that it
clears depth to run before geometry and writes `frag_depth`, disabling
early-Z — is real and worth about a millisecond. The raymarch was
everything else.

⚠️ **They are switched off, which is not an optimisation.** Any project
that sets coverage above zero pays 39 ms again, and nothing warns. #731
is the condition on them coming back.

### 🟢 #780 is measured, and it is not the problem — 2026-08-14

Four passes — z-slice, count, allocate, populate — and `inti_shade`
walks the lights of its own froxel. The claim is now a number: the
allocation pass counts the peak and the mean per cell and rides them
home in the readback it already runs.

| | lights |
|---|---|
| reaching a point (from the scene file) | median 12 · max 14 |
| **the busiest froxel holds** | **26** |
| **the mean froxel holds** | **14.9** |

**~24 % over-listing**, which is what conservative cell-vs-sphere
assignment costs and is ordinary for the technique. `cluster grid` is
0.15 ms of a 31 ms pass. The grid is doing its job.

⚠️ **Three predictions failed here, each argued from geometry rather
than measured**: that lowering `far` would thin the froxel usefully (it
bought 10 %), that raising `first_slice` would do better (it made the
frame far worse — the scene sits nearer than 20 m, so all of it fell
into slice 0), and that over-inclusion was 2.9× (it is 1.24× at the
mean; the 2.9× came from bisecting a colour ramp by eye). The counter
settled all three, which is why it exists.

🔴 **And the tool has to live where the measuring happens.** The first
version of the shading-LOD control shipped in the editor only, where the
whole raster pass is 0.12 ms and switching every specular layer off
moved it by 0.001. `KOOCH_CLUSTERING` already carried that lesson in its
own doc comment.

### What was ruled out, so it is not revisited

Four families were considered against the measurements and dropped. The
reasons are worth keeping, because every one of them is a technique
somebody will suggest again.

| | why not |
|---|---|
| **Lightmaps / irradiance volumes** | The cheapest answer by far — a hundred static lights become one texture fetch — and **static by construction**. Ruled out by a product decision: these lights move |
| **ReSTIR DI / Bevy Solari** (#796, #819) | Solari's world cache alone costs 2.65 ms per refresh in Bistro *on the author's machine*, against our 13.9 ms total. Denoising goes through DLSS Ray Reconstruction; the 890M has no equivalent. Its author calls the design unsatisfying and light sampling is unshipped |
| **UE5 MegaLights** | Requires hardware ray tracing, and exists for **shadow-casting** lights. `many_lights.scene` sets `cast_shadows: false` on all hundred, and `shadows` is 0.7 ms of a 31 ms pass |
| **Godot VoxelGI, Lumen, DDGI, radiance cascades, SSGI** | All solve **indirect** light. This engine has no GI and does not pay for one — they *add* to the 31 ms rather than subtracting. Godot's own docs say VoxelGI "is not suited to low-end hardware such as integrated graphics", which is precisely the target |

⚠️ **Voxel injection is the exception worth naming.** VoxelGI does two
things: it injects lights into a grid, and it cone-traces through that
grid for bounces. The first half is a pixel taking one sample instead of
walking fifteen lights — useful. The second half is what makes it
expensive, and it buys bounces nobody asked for. #826's third option is
that first half alone.

### 🔴 What is left is the lights that do reach — and it is not ALU

`SpecularFloor` / `KOOCH_SPECULAR_FLOOR` (#821) skips the specular layer
— GGX `D`, Smith `V`, Schlick `F`, the multiscatter fit, the
representative point — for a light whose irradiance at the pixel is
below a threshold. Measured on the OneXFly, through Steam → gamescope,
with **every** light forced to diffuse-only:

| | `GPU > raster + shade` |
|---|---|
| baseline | 31.41 ms |
| every light diffuse-only | 28.28 ms |

**A 10 % ceiling.** Deleting the expensive half of the BRDF fifteen
times per pixel bought 3 ms of 31, so **that pass is not ALU-bound**.
What is left: the 15 `IntiLight` records fetched per pixel, and fill
rate — which matches the frame falling 5.2× when the internal
resolution drops.

That is the argument for **#796 / #819**: ReSTIR does not make a light
cheaper to evaluate, it makes the pixel evaluate one or two instead of
fifteen. It attacks the axis these experiments left untouched, and they
are the evidence that the other axis is nearly empty.

What it deliberately does not do: read back the furthest light to size
the grid's far plane, the way Bevy does. That is a readback in the hot
path. `ClusterSettings::far` is a setting instead, and a light past it
piles into the last slice — conservative, never wrong, just not saving
anything out there.

### What #780 had to copy, and what it must not

Read off `pbr_functions.wesl:453` rather than from memory:

```wgsl
let cluster_index = clustering::view_fragment_cluster_index(frag_coord.xy, view_z, is_ortho);
var ranges = clustering::unpack_clusterable_object_index_ranges(cluster_index);

for (var i = ranges.first_point_light_index_offset;
         i < ranges.first_spot_light_index_offset; i = i + 1u) { ... }
```

- The z-slice is **logarithmic** in perspective —
  `log(-view_z) * factors.x - factors.y + 1.0` — so clusters follow how
  depth precision falls off rather than metres.
- 🔴 **Ranges per object type, not one light list.** Point, spot,
  reflection probe, irradiance volume and decal each get a consecutive
  range, so the loop carries no per-type branch and a shader that needs
  no decals never walks them. Building it as "lights per cluster"
  produces half of it, and the other half is a rewrite.
- Two flags are checked before any fetch: one on the mesh, one on the
  light. The mesh half is done (#804); the light half is not.

⚠️ **Build the grid once.** #477 (VSM page marking), #731 (the fog
froxel buffer) and the single shadow-page pool all want this same
structure. Three grids is the failure mode.

### 🎯 The frame on the OneXFly is fill-rate bound, 2026-08-12

Roll A Ball, release build with `kooch/profiling`, on the handheld,
captured live off the game's own `puffin_http` server. Same scene, same
binary, same Steam → gamescope launch; the only thing changed between
the two rows is the internal resolution:

| internal res | frame | `vkAcquireNextImageKHR` | `frame` (the engine) | `gpu_busy_percent` | sclk |
|---|---|---|---|---|---|
| native, fullscreen | **72.17 ms** | 68.53 | 2.75 | **96 %** | 1144 MHz |
| `-w 640 -h 360` | **13.91 ms** | 11.56 | 1.68 | 70 % | 1417 MHz |

**The frame collapses 5.2× with the pixels.** The cost is per-pixel, and
that is a finding, not a suspicion.

🔴 **The wait is not the compositor.** `vkAcquireNextImageKHR` blocks
when no swapchain image is free, and an image is not free while the GPU
is still drawing into it. At 96 % busy the acquire is where the CPU
learns the GPU is behind — the thermometer, not the fever.

🔴 **It is not the CPU either.** `frame` — the whole engine, cull,
shadows, raster and shade — measures **~2 ms in every condition
tested**, fullscreen or windowed, fast frame or slow. Physics, update
and input together stay under half a millisecond.

🔴 **The low clock was a symptom.** 1144 MHz of 2900 at native
resolution, and 1417 MHz at 640×360 — the GPU clocks *higher* with less
work, because at native it spends the frame waiting rather than being
throttled. At 640×360 the game is no longer GPU-bound at all: 13.91 ms
is the 72 Hz vsync ceiling with 30 % of the GPU to spare.

**The suspect is #771, the sky** — 8 192 hash evaluations per pixel,
paid on pixels the geometry later covers, with `frag_depth` disabling
early-Z. It is the only candidate whose arithmetic scales exactly like
this. But per-pixel cost is also what the fused shade pass and the
shadow passes have, so **which** pass owns the 70 ms is still open, and
that is now the whole job of the GPU scopes in #785.

⚠️ **Still not established: 10 W on battery.** Every number here was
taken plugged in. The target is 72 FPS at a third of the power.

#### How to measure this, so the next measurement is not thrown away

Four measurements were needed because the first three measured the wrong
thing, and the mistakes are cheap to repeat:

- 🔴 **Launch the game the way it ships — through Steam, into gamescope,
  fullscreen.** The same binary run over SSH as a loose 1280×720 wayland
  window reports **13.9 ms and a healthy GPU**, because gamescope never
  treats it as a game: no focus, no fullscreen, no scaling to the panel.
  Measuring outside that path does not measure the game.
- 🟢 **`gpu_busy_percent` in `/sys/class/drm/card*/device/` separates
  GPU-bound from waiting in one command**, with nothing instrumented. It
  is the first thing to read, before any hypothesis.
- 🟢 `KOOCH_PRESENT_MODE=novsync` takes the vblank out of the frame time,
  and `KOOCH_FRAME_METRICS=log` prints frame, CPU and GPU milliseconds
  per second to stdout — no panel, no network, no profiler.
- 🟢 **The profiler itself is trustworthy and free**: a capture taken
  live and the game's own log agree to the hundredth of a millisecond in
  the same run, and draining 15.2 MB off the server in 25 s does not move
  the frame time. `server.rs` drops frames rather than blocking when a
  client falls behind.

### #769 — where the frame actually goes

Half of this is now answered: the frame is **GPU-bound and per-pixel**,
and the engine's CPU side is ~2 ms. What is missing is the split *by
pass*, and the same measurement at 10 W on battery.

The budget still has to be *divided*, and nobody knows that split. One
pass can be obviously wasteful and fixing it can still leave the frame
nowhere near 13.9 ms.

What comes out is a table — pass, milliseconds, share of 13.9 ms — and
that table sets the order of every performance issue after it. Measured
on the device at 10 W: frametime *and its distribution* (an average hides
a stall, and a stall is what a 72 FPS target fails on), GPU timestamps
per pass, CPU vs GPU bound, the resolution actually in use, the adapter
actually selected, and the clocks while it runs.

🔴 Guessing has a bad record here. Three hypotheses about a rendering
problem failed in a row in the shadow work, and one `eprintln!` of both
sides ended it. This issue closes when there are numbers.

### #771 — the sky, which does not need a profiler to justify opening

Counted from `sky_main.wgsl` rather than measured: up to **8 192 hash
evaluations per sky pixel** — 32 primary steps, each firing a 3-step
light march, each sample evaluating two 4-octave FBMs of 8 hashes each.

🔴 **And it is paid on pixels that end up hidden.** The sky pass clears
colour *and* depth, so it runs before the geometry: every pixel looking
up marches the whole slab, and then the ground draws over it. The
fragment shader also writes `frag_depth`, which disables early-Z — so
reordering the pass buys nothing until that goes too.

The constants are already tuned to the edge of visible banding (48 → 32
steps, 4 → 3 light steps, 800 → 500 length). **There is nothing left to
take.** The cost is structural: procedural FBM per sample, where
everything else in the industry does a filtered 3D texture fetch. The
direction is Guerrilla's *Nubis*, which #731 already cites.

⚠️ Reduced-resolution rendering with temporal reprojection is the other
half of that technique, and it needs motion vectors — **which do not
exist** (#732).

### The scale ceiling — filed, and deliberately not scheduled yet

The Bevy sweep put these two above everything else on its take list, and
they existed only inside a research document for four days. That is how
#737 was lost, so they are issues now.

| | | |
|---|---|---|
| **#773** | the visible id is `(instance << 16) \| meshlet` | 🔴 65 536 instances, guarded by a `debug_assert!` that **release compiles out** — instance 65 537 aliases onto another one, silently. The packing is written six times, and a shader left on the old shift decodes garbage rather than failing to compile |
| **#774** | meshlet BVH culling (Bevy 0.17, PR #19318) | Render cost becomes nearly independent of scene geometry — 115 billion triangles in 3.5 ms on a 4070. Needs #773 first: culling cheaply is pointless while the id cannot name what it culled |

**Why not next**: this is the ceiling that stops a *planet*, not the one
costing milliseconds this week. Roll A Ball does not have 65 000
instances of anything. ⚠️ And #774 is the item most likely to behave
differently on a handheld than in Bevy's benchmark — a BVH walk is
pointer chasing, and an iGPU's scarcity is bandwidth. Measure the walk on
the device before committing to a layout.

Reading #774 may also **retire** work rather than add it: Bevy shipped
two-phase occlusion culling in 0.16 and absorbed it into BVH culling in
0.17, so our parked Hi-Z two-pass (#486 / #445) may want to be a phase of
it rather than a neighbour.

### Further out

🔴 **#477 (VSM) will walk into the same set of problems #476 fixed.**
Every one of its nine was an orthographic view doing what a perspective
one does not; a virtual shadow map is more orthographic views, not
fewer. **And it has to fit the budget**, which is a question nobody has
asked of it yet.

### Still open from the build work

**#767** choose the game's first scene — `main_scene` is in
`project.kooch`, the editor writes it, and the runtime never reads it.
**#766** input repeats in Play. **#763** shorter names.

### Cross-compilation: measured, not assumed

Toward `x86_64-pc-windows-gnu`, on the author's machine:

| | |
|---|---|
| `kooch_core` | 🟢 green in 11 s |
| `metis-sys` | 🟢 green **with** `CFLAGS_x86_64_pc_windows_gnu="-std=gnu17"` — mingw-gcc defaults to C23, where `false` is a keyword, and GKlib declares an enum by that name |
| `meshopt` | 🔴 needs `mingw64-gcc-c++` installed |
| macOS | ❌ impossible without a Mac; Apple's SDK is not redistributable |

The build panel has to pass that `CFLAGS` and detect a missing toolchain
**before** compiling. Bevy's answer to the same problem is a matrix of
native runners rather than cross-compiling — which is #753, and the
other half of this.

---

## The tooling that had to be fixed to measure any of this, 2026-08-13

None of it was planned. All of it was in the way.

**The version had never moved off 0.1.0** — no tags, no releases, every
crate reading the same number since the workspace was created. Now
**0.2.0** (#798), which is a MINOR because `BuildPreset` changed shape.
⚠️ Still no tag and no release: a version number nothing points at is
bookkeeping, and #756 (self-update) cannot exist without one.

🔴 **And it has to move with every change to the engine, not once a
release** (2026-08-13). The editor compares the engine a project builds
against with the one it ships, and with the number standing still the
only thing it could say was *"same version, different source"* — an
alert that fires on every open, says nothing actionable, and gets
dismissed without reading. A version that does not move is not
bookkeeping, it is an alert that cries wolf. **0.2.1** is the froxel grid
(#780) and the main-scene fix (#808).

**Build presets became one dropdown.** `release` / `profiling` /
`runnable` are gone; `mode` is **Release** or **Profiling**, both
optimised — LTO and one codegen unit, set through `CARGO_PROFILE_*` in
the environment rather than a project's `Cargo.toml`, which is generated
once and would have skipped every existing project. 🔴 There is
deliberately no debug mode: the editor's own debug build measured
14.31 ms a frame against 4.94 release, so profiling one describes that
build and not the game.

**Install was broken twice, for unrelated reasons.** It asked for the
*project's* engine version, which `ensure_current` answers with whatever
is already on disk (#799) — and then never recorded the move, because
**two files hold that state**: `Cargo.toml`'s path and `project.kooch`'s
`engine_version` (#801). The prompt returned for ever and nothing failed.

**The launcher now sets a project's engine before opening it** (#802).
The order was backwards: opening compiles the project's plugin and
*then* compares versions, so a mismatch cost a compile against the
engine being left behind and produced an `.so` that `BuildStamp`
refuses. `move_project_to_engine` is now the only writer of either
record, and moving a project must not build it — there is a test that
asserts no `target/` appears.

**GPU scopes exist** (#795): `wgpu-profiler` bridged into puffin as a
`GPU` thread. 🔴 Its own `puffin` feature is unusable here — it wants
puffin ^0.19.1 against this workspace's patched 0.20 — so the bridge is
45 lines adapted from its `src/puffin.rs`. That also saved the *game*
build, which is the root of its own workspace and inherits no
`[patch.crates-io]`.

⚠️ **Two scopes were reporting other people's time**, found by printing
the capture as a tree: a `profiling::scope!` lives to the end of its
block, so declared mid-function `upload instances` claimed 1.900 ms of
which 0.031 was the upload. Both are braced now.

### 🔴 Five captures, and none of them comparable

Every one came from a different camera or scene, so differences mixed
the change being tested with what was on screen — 11 ms were attributed
to "removing the sky" that were the viewpoint. Only the three
resolution captures were a real A/B, and they are the only ones any
conclusion here rests on.

**Change one thing. Same view. Same scene.** And check the panel before
saving: a capture that reads `scope#ScopeId(137)` lost its names and can
only be read by inference.

## After lighting — the areas the user named, 2026-08-10

Not scheduled against the budget yet; written down because the order is
decided and what is not an issue evaporates.

| | | Note |
|---|---|---|
| ~~#785, in the editor~~ | ~~a profiler panel~~ | **Done** (PR #790). Adopted, not written: puffin + `puffin_egui` + the `profiling` facade. See below — the rest of #785 moved up into the queue above |
| **#784** | shader graph, the Shader Forge clone | #440 already built the half underneath: a graph compiles to the material *body* `compose_material_shader` concatenates |
| **#732 / #536 / #481** | temporal upscaling — FSR, DLSS, XeSS | Already filed. FSR first: it is the one that runs on the OneXFly, and an untested fallback is a broken fallback |
| **#477** | virtual shadow maps | 🔴 The user's call: encarar it **with** the ray tracing Bevy has (Solari), not before. See below |

### What the profiler already changed, 2026-08-11

It answered its first question before it was finished, and the answer
moved work off the queue rather than onto it.

```
release, 1237 frames   median 4.94 ms   p99 8.25   max 9.10
Surface::get_current_texture   2.724 ms/frame   ← 55%, and it is WAITING
frame                          0.692 ms  × 2    ← the whole engine, per viewport
SkyRenderPass::render          0.008 ms  × 2
```

- **The editor has no performance problem in release.** The largest entry
  in the frame is the wait for the compositor. The engine renders a full
  viewport — cull, shadows, meshlets, shading — in **0.69 ms**.
- **Debug against release was 14.31 ms against 4.94.** Every impression
  of slowness during the session came from an unoptimised binary.
- ⚠️ **`frame` runs twice per editor frame.** The View and Game panels
  each render the scene with their own cull and shadow passes. Real, and
  nothing to do with the profiler.
- ⚠️ **The sky costs 0.008 ms *here*.** #771's 8 192 hashes per pixel is
  a resolution-and-device claim, and this measurement cannot confirm or
  deny it. It is the handheld's to answer.

🔴 **And it is why the rest of #785 moved into the main queue.** Every
number above describes a desktop.

🟢 **It reaches the handheld now** — the capture at the top of this file
came off the OneXFly over the network, and the CPU tree it carries is
complete: every `kooch_render` scope is there, named, down to
`raster + shade (fused)`. ⚠️ An earlier note here claimed `Render`
arrived as one opaque box and that the sky scope was missing from the
game's capture. **Both were misreadings of the panel**, corrected on
2026-08-12 by dumping the capture file directly.

What is genuinely missing is the other axis: **the CPU tree cannot
attribute GPU time**, and the frame is GPU-bound. That is what the GPU
scopes are for, and #769 still cannot be written without them.

### Why VSM waits for ray tracing

The reframing came from the #777 smoke, and it is in #782 and #477:
cascades are described as the directional light's solution, but they fit
the **camera's** frustum, not the light. That argument never mentions
the light type — so at planetary scale, where a star is a point light
with no useful `range`, every kind needs detail apportioned by distance
to the viewer. Generalised, that is a virtual shadow map.

Which puts #477 and #782 in the same place: the same question at two
scales. And both land near the technique Bevy is building its ray traced
lighting on, so doing them together is what stops three mechanisms from
being designed against each other.

🔴 Three places now where Bevy's source stops being the ceiling: shadows
at planetary scale (#782), virtual shadow maps (#477), and a node
material editor (#784). Everything else in the lighting port is a port.

### One page pool for every shadow — the user's framing, 2026-08-10

> *"después vamos a hacer que todas las sombras usen un mismo mapa de
> sombras así resulta más barato el virtual shadowmap"*

Right, and it is the premise of a virtual shadow map rather than a step
before one. Today there are two allocations, each sized for its own worst
case, each paid whether or not anything uses it: the cascade + spot array
at 2048² × 8 layers (**128 MiB**) and the point cubes at 512² × 24
(**24 MiB**). **152 MiB standing** in a frame that may contain no
shadow-casting light at all.

A shared pool does not change the total — it changes **what the total is
a function of**. Pages go to whichever light needs detail *where the
camera is looking*, so the budget follows the screen instead of the sum
of every light type's worst case. That is UE5's design, and it is why the
Chalmers papers reach hundreds of casting lights with bounded memory.

🔴 **It is also the strongest argument for #780 landing before #477.**
The pool has to be handed a list of which lights need pages, at what
resolution, where — and that list is what a cluster structure produces.

⚠️ Until that decision: #778's 512² face size and the cascade array's
layout are **provisional** and must not become public settings. A setting
is a compatibility promise.

## Done — #758, and a game that runs on a machine that is not this one

The thing that paused the graphics work: nothing here had ever produced a
game. It has now.

- **Build panel with presets per target** (#758). A `.buildpreset` is a
  reflected asset, so the Inspector edits it with no UI code (#744). The
  build can be cancelled.
- **Encrypted asset packs.** zstd + AES-256-GCM with the index sealed
  too, so the file does not reveal the *names* of what is in it. Scenes
  go in the pack — a scene is the structure of the whole game, and
  leaving it in plain text beside an encrypted pack protects the textures
  and publishes the design.
- **Only what the game reaches travels.** Packaging walks the scenes and
  prefabs for GUIDs; source does not travel, and neither does the preset.
- **The shipped build is the game only** (#558), asserted by the
  packaging tests rather than trusted.
- **The editor ships as one `.AppImage`** that materialises its own
  engine for projects.
- 🔴 **`min_glibc`** — a game built here refused to start on the OneXFly:
  glibc is forward compatible and not backward, 2.43 against 2.42. Now
  routed through `cargo-zigbuild`, which needs no root — which matters on
  an immutable distribution. See [Shipping a Game](book/src/guide/shipping.md).

Two things that took longer than they should have, both worth
remembering. The engine's own materials failed to resolve in the packaged
game because GUIDs were compared in two different spellings — three
guesses lost, one `eprintln!` of both sides won. And `.exists()` was used
twice, in two files, to decide a packaged game's layout: a packaged
game's files are not on disk.

⚠️ It runs, and it runs badly. That is the constraint at the top of this
file, and #769 is where it goes next.

## Done — #754, the engine lives once per machine

A compiled editor can now create projects that build. The generated
manifest used to carry an absolute path to the engine clone of whichever
machine created it.

- **One engine per version**, in `~/.local/share/kooch/<version>/engine`.
  Nothing inside the project; two versions coexist so a project pinned to
  an older engine keeps building.
- **The licence is mandatory by construction.** The facade does
  `include_str!("../LICENSE.md")`, so it is inside every executable that
  links the engine — not a file anyone has to remember to copy.
- **No test code leaves the repo.** 237 modules moved out of source
  files; the vendor filter reads `#[cfg(test)] mod X;` declarations
  rather than guessing at filenames, because three of the engine's test
  files are called `measure.rs` and `id_stability.rs`.
- `examples/package_editor` produces a distributable editor, verified as
  a 123 MB AppImage that creates buildable projects.

🔴 **Why the source is on disk at all**: Rust has no stable ABI, a
precompiled `rlib` links only against the exact compiler and dependency
versions that built it, and cargo does not model binary dependencies. No
Rust engine ships binaries. The source is protected by licence, the way
Unreal protects theirs. See [ADR 0002](decisions/0002_infinite_reverse_z.md)
for the other decision this stretch produced.

⚠️ **Rust remains required** on the editor's machine. Gameplay is native
Rust compiled into the game; no packaging fixes that.

---

## Done — #735, contact shadows

A short ray marched through the depth buffer, from each shaded point
toward each light that opted in. Ported from Bevy 0.19
(`contact_shadows.rs` + `calculate_contact_shadow`, over their
`bevy_pbr::raymarch`, itself Tomasz Stachowiak's `raymarch.hlsl`), with
their defaults kept: **16 linear steps, 0.1 m thickness, 0.3 m length**.

**What it does that the cascades cannot.** A cascade is correct at range
and worst exactly at contact — the few centimetres where an object meets
the ground is where its shadow detaches or swims, whatever the bias. The
march fixes that band and nothing else, so the two compose. And being
screen-space it **costs the same at any world scale**, which is rare in
this backlog.

### 🔴 The one thing that was NOT a straight port

Bevy's march reads depth as `1.0 / ndc_z`. That identity holds **only
for a reversed-Z projection with an infinite far plane**, and
`perspective_rh_reverse_z` takes a finite one. Ported literally it would
have scaled every depth by a factor that varies with the far plane —
i.e. a `thickness` parameter meaning something different in every scene,
which is exactly the kind of wrong that looks like "needs tuning".

`contact_shadow::depth_to_linear` inverts the real projection instead:
`linear_z = x / (y − ndc_z)` with `r = far/(near−far)`, `x = r·near`,
`y = 1+r`. A test round-trips points through the actual matrix, and a
second one pins that as `far → ∞` it collapses back onto Bevy's form.

### Where it lives

| | |
|---|---|
| The march | `kooch_render/shaders/contact_shadow.wgsl`, bindings templated |
| Its uniform + settings | `kooch_render/src/contact_shadow.rs` |
| The call | `inti_shade`, per light, skipped where a cascade already shadows |
| Per-light opt-in | `GpuLight.flags` bit 0, from `*Light::contact_shadows` |
| Author settings | `RenderSettings.contact_shadow_*`; **zero steps is the off switch** |

`inti_pbr.wgsl` **calls a function it does not define**, and that is the
mechanism rather than an oversight: a shading path that forgets to
concatenate the march fails to compile instead of quietly rendering
without it. `INTI_CONTACT_SHADOW_STUB` is what a path with no depth to
sample supplies.

Both bindings sit in **group 0**, not Inti's — the depth buffer is per
view and that group is shared across views, which is what made shadows
vanish once the light buffer grew.

### Defaults, and why they differ per light kind

On for `DirectionalLight`, off for `PointLight` and `SpotLight`. A scene
has one sun and can have fifty lamps, and the cost is per light per
pixel: it is the lamps that need the opt-in and the sun that needs to be
seen. ⚠️ On a punctual light this is currently the **only** shadow it
casts.

### 🔴 The test that caught the two paths diverging

`try_acquire_device` asks for `Features::empty()`, so the shared test
device **cannot** take the R64 path — every render in `contact_shadows.rs`
would have gone through the R32 compute deferred and the fragment path
would have shipped untested. The file acquires a second device with the
int64-atomic bundle and asserts the same property on both. That is the
#476 failure mode caught before it happened rather than after.

The march itself was wrong on the first run — a `select` in the frustum
clip was inverted, and the test failed with the two renders **bit
identical**. It is a test that fails when the feature is absent, which
is not true of every test this repo has written.

### Known limits

- **Screen-space**: an occluder off-screen or behind the camera does not
  exist. The ray is clipped to the frustum and reports no hit, so the
  shadow fades at the screen edge rather than popping.
- **No temporal filter.** The jitter is per frame, and without TAA
  (#732 — motion vectors do not exist) a still frame keeps its dither.
- The blown-out floor (#254) makes the effect harder to see than it is.

---

## Done — #476, the sun casts

**#476 is done: the sun casts.** The pieces that sat on the branch
untested-in-anger — cascade placement, the atlas, the depth pass, the
PCSS filter — are now built by `MeshletRenderStage` on the first frame
that finds a `DirectionalLight` with `cast_shadows`, and released again
when nothing wants one. `tests/csm_shadows.rs` is the acceptance:
**a cube over a floor darkens the floor beneath it, and does not darken
the floor beside it.**

The second half of that sentence is the one worth keeping. Every
reversed-Z mistake in this feature — an inverted sampler, a clear to 1.0,
a positive bias — darkens *everything*, and a test that only checks "the
shadow is dark" passes on all of them.

### What the author controls

`.rendersettings` grew three fields, so the atlas is not a constant
someone has to recompile to change: `shadows_enabled`,
`shadow_distance` (metres — the cascades are fitted to whatever range
they are given, so raising it blurs the shadows near the camera rather
than adding distant ones), and `shadow_cascade_texels` (the atlas is
twice this per axis: 2048 costs 64 MiB, 1024 costs 16).

### Known limits, stated rather than discovered

- **Alpha-cut geometry does not cut.** No fragment shader in the depth
  pass means foliage casts the shadow of its quad.
- **Only the directional light casts**, and only the first one. #734.
- **The pass runs per view.** Two open panels are eight cascade culls and
  eight depth passes a frame.
- **No temporal filter.** Bevy's third option (Jimenez '14) rotates its
  taps by per-frame noise and needs TAA to resolve it — #732, and motion
  vectors do not exist yet. Castano '13 is what they ship by default and
  what runs here.
- **`unclipped_depth` is not emulated.** Where the adapter lacks
  `DEPTH_CLIP_CONTROL` the near plane falls back to a cascade-width
  margin, which costs depth precision. Bevy emulates it in the fragment
  shader instead.
- **No test pins the scaled-instance LOD.** The property is a survivor
  count out of the cascade culls and `ShadowPass` does not expose them.

### What the port cost, and the pattern in it

Nine fixes after the pass was wired, and **every one was a place where
Bevy 0.19 does something this engine had not needed before shadows
existed** — an orthographic view. The LOD selector divided by a distance;
the splits anchored at the camera's near plane; the cull cone-tested from
a viewpoint a directional light does not have; the simplification error
ignored instance scale; the slices touched instead of overlapping.

The recurring shape: a mechanism ported in half. The cascade blend went
in without the slice overlap it needs, and produced an artifact neither
half has alone.

🔴 **And the diagnosis pattern, which cost the most:** three times a
cause was named from a screenshot and three times it was wrong. What
worked was the `Shadow cascades` debug view, which separates "the map is
incomplete" from "the sampling loses it" — two failures that look
identical in a shaded frame and live in different files.

### Next, and why it is contact shadows

Cascades are worst exactly at contact — the few centimetres where an
object meets the ground is where a shadow detaches or swims, and that is
what makes things look like they float. Screen-space, so it costs the
same at any world scale.

Then **#743**, the debug views the owner actually asked for: one light at
a time, greyscale, **with its shadow**. It answers *why is this dark* —
no light reaching it, or a shadow reaching it — which look identical in a
final frame and have different fixes. It was blocked on #476 and is not
any more.

Then #250/#248 atmosphere, and #254 post + auto exposure.

## Also open, from the same stretch

- **#745** — 🔴 the colour picker edits sRGB and writes into linear
  fields. **Every authored colour is ~2.3× too bright**, in every light
  and every material, and has been for as long as both existed. Invisible
  until #441 made colour mean something. Cheap to fix.
- **#746** — an eyedropper, deliberately blocked on #745: a pixel is sRGB
  post-tonemap, and an eyedropper *feels* authoritative.
- **#736** — user settings, now scoped against #744: what the player
  picks, under `~/.config/`, not committed.

---

## The scale work, which the goal makes unavoidable

**Everything is judged against universes**: planetary and galactic draw distance, detail only up
close, far away merely *distinguishable*.

🔴 **Our visible-meshlet id packs as `(instance_id << 16) | meshlet_id`** — a hard ceiling of
**65 536 instances**. A single vegetated chunk exhausts it. Bevy removed its comparable 2²⁴
cluster limit in 0.17 and now renders 115 billion triangles in 3.5 ms.

1. **Widen the id packing.** Nothing else on this list means much until scenes can exceed 65 k
   instances.
2. **BVH culling over clusters** ([bevy#19318](https://github.com/bevyengine/bevy/pull/19318)) —
   render cost becomes nearly independent of scene geometry.
3. **Visibility Ranges / HLODs** — per-mesh appear and disappear distances. The honest answer to
   "far away only has to be distinguishable".

Neither has an issue yet.

**Adjacent, and now filed:** **#732** (temporal upscaling) opens with the prerequisite nobody can
skip — **we have no motion vectors, no jitter, no temporal history**, verified by grep. That is
one piece of work with three consumers: upscaling, TAA and motion blur. `dlss_wgpu` is a
standalone crate usable without Bevy, but **FSR comes first**, because it is the path that runs
on the developer's own handheld and an untested fallback is a broken one.

---

## Two gaps with no issue, found by inventory rather than by use

Both block the goal, and neither will announce itself until it does.

- **No task pool.** Loose `thread::spawn` in `frame_pacing`, `runner` and the editor's remote
  session; no pool, no cancellation, no priorities. Streaming a planet means loading off the
  frame, and there is no mechanism for it.
- **No app state machine.** Menu, loading, playing, paused. Its absence is also why
  `ActionMap`'s bulk enable/disable has no consumer.

Cheap and adjacent: **an infinite grid** — shader-drawn ground plane with distance fade, no
geometry, correct at any scale; the viewport has no ground reference at all. And **no colour
type**, which is a silent hazard: wrong-space blending looks almost right until it is wrong
everywhere at once.

### Filed from the Bevy sweep

| | |
|---|---|
| **#731** | Volumetric clouds as a **spherical shell** per planet. Bevy's fog volumes are AABBs, which compose with chunks and not with a planet. Shares the ray-sphere maths with #248 |
| **#732** | Temporal upscaling on both vendors — motion vectors first |
| **#733** | Spike: hot-patch a project's Rust without reopening, via Dioxus' `subsecond`. Aims at a friction that is already documented: the editor loads the project `.so` and does not build it |
| **#734** | Light textures — cookies and gobos |
| **#735** | Contact shadows |
| **#736** | One settings framework. We have three shapes today — `EditorConfig`, the dock layout, and nothing for a game |

**#248 grew three things** it did not have: the LUT-per-planet scale problem (a solar system needs
one parameter set per planet, and a galaxy needs a policy), [bevy#20766](https://github.com/bevyengine/bevy/pull/20766)
as the reference for the spherical raymarch, and 🔴 **0.18's generalised scattering media — the
part that makes non-Earth planets possible, and which has to be in `.atmosphere_material` from
day one** rather than retrofitted around a shader written for Earth's three-term model.

**#453 grew the reason skinned meshes vanish**: they are culled against the **bind pose**, so a
reaching animation leaves the volume and the character is culled while still on screen. Reads
as a streaming bug for a long time. The GPU skinning pre-pass is where the animated bounds are
free.

### Three more, cheap, that make whole classes of bug unrepresentable

- **Required components.** Bevy deprecated bundles in 0.15 for this: a component declares what
  it needs, and inserting it inserts them. We have no such mechanism — `AddComponentCommand`
  inserts exactly one type. So adding a `MeshRenderer` to an entity without a `Transform`
  produces an entity the renderer's query never matches: **authorable and inert**, the same
  family as the lights in #441 and the nine capabilities #726 catalogued. This does not make the
  bug easier to find; it makes it impossible to author.

- **Easing and curves as a shared type.** They already exist — `kooch_camera::blend::{eased,
  ease_in}` and `virtual_camera::ease` — **locked inside the camera crate, addressed by `u32`**.
  Scene animation (#715), the animation graph (#717), UI transitions (#96) and any tween will
  each write their own unless this moves out. Bevy's `EaseFunction` is 39+ variants implementing
  `Curve<f32>` with sampling, clamping, composition and reparametrisation — the shape to copy,
  not the code.

- **A colour type.** See above: sRGB is touched in the texture loader and the GPU context and
  nowhere else.

See `docs/research/bevy_module_gap_2026-08-05.md` and
`docs/research/bevy_feature_sweep_2026-08-05.md`.

### Already ours, for the record

**Commands** are complete — `spawn`, `spawn_batch`, `entity`, `despawn`, `insert`,
`insert_reflected`, `apply`, with `EntityBuilder` and `EntityCommands`. `spawn_batch` is
specifically what Bevy recommends for mass spawning, which is what streaming a planet is. There
is nothing to port. **Bundles** should not be ported at all: they are the deprecated half of
that pair.

---

## The render graph — decided, not yet executed

**#392 resolved: delete it.** Not because a scheduler is wrong, but because the right scheduler
is the ECS one. Bevy — the engine that made render graphs the default pattern — **replaced
theirs with ECS schedules in 0.19**, because their graph ran as an exclusive, single-threaded
system. Passes are systems there now.

Half the replacement is already written here: `kooch_core::schedule::gpu_batch` batches GPU
systems into one shared encoder, and the `PreRender` / `Render` / `GpuSync` / `Gpu` stages
exist. What is missing is **`before` / `after` ordering between systems inside a stage** —
engine-wide, not render-only.

`sky/node.rs`, the graph's orphan adapter, is already gone.

---

## Done recently

| | |
|---|---|
| **#592 — a frame is a list of views** (PR #730) | `MeshletRenderStage` holds a `SlotMap<ViewId, MeshletView>`; the editor grew a **Game panel** rendering the gameplay camera beside View. Cull buffers, Hi-Z state and `group_max_err` moved per view — sharing them was [bevy#15182](https://github.com/bevyengine/bevy/issues/15182) waiting to happen. Play no longer switches the editor camera off, `input_focus` decides who owns input in one place, and `ViewCamera` replaced four copies of "walk the world for the highest-priority camera" |
| **#727** | **An input action became an asset.** A `.inputaction` holds one action with its own id; a component points at it by guid; `enabled` is per action. The map — `InputMapSource`, the `.inputmap` asset, `ActiveActionMap`, `ActionState`, the generic `ActionMap<A>` — is gone. #55 and #58 closed |
| **#728** | **A saved asset reaches the running project.** Only prefabs ever told it; a material edit updated the editor and left the game rendering the old one. `forget` + `load` cannot do this — `insert` mints a new key and every live `Handle<T>` keeps the old bytes — so `reload_path` writes over the slot instead. No file watcher, on purpose |
| **#711** | **`kooch_input` was connected to nothing.** No backend was ever constructed and `WindowEvent::KeyboardInput` only asked for a redraw. `just_pressed` was permanently false, and four green tests were pinning that |
| **#713** | **A game can be played inside the editor.** The editor captures input and sends snapshots to the headless host over the protocol. Identifiers became this engine's own — `gilrs::GamepadId` has no public constructor, which is what blocked it |
| **#718** | A camera follows a **`CameraTarget` tag**, not an entity reference. Several tagged entities *are* a group, so following one and framing four are one code path |
| **#723** | A project's component had no fields in the prefab inspector — three sources answer "what fields does this type have" and that panel asked the only one it was not in |
| **#724** | "Open in IDE" opens the project root, in the IDE this machine has. Four bugs, each exposed by the previous fix |
| **#669 phase 0** | **Done.** A ball rolls under WASD and a stick, camera-relative; a raycast gates the jump; a virtual camera follows. Verdict in the issue |
| **#605** | The custom ECS stays. `bevy_ecs` evaluated on measurements and declined |
| **#607** | Stable entity references — a component can point at an entity and survive a save |
| **#609** | More than one scene loaded at once |
| **#612** | Parented physics — compound colliders, gizmo parent-space conversion, Inspector warnings |
| **#560** | Joints — one `Joint` component, all eight rapier kinds, motors, limits, breaking |
| **#618** | Mass control — `mass` means kilograms, shapes are massless, explicit centre of mass |
| **#563** | Physics debug render — the solver's own account of itself, in the viewport |
| **#623** | Collider material — friction, restitution, combine rules and damping, all authorable |
| **#611** | **Prefabs, both phases.** A prefab is a scene file; an instance in a scene is a *reference* to it plus what the user changed. #676, #677, #678, #679 |
| **#661** | The keyboard belongs to the focused panel, not to World |
| **#561** | Collision events, sensors and groups — the solver can finally report back |
| **#630** | Event delivery — `Events<T>` had never been rotated by the editor's runner |
| **#635** | A Console tab, and structured project logs to put in it |
| **#624** | Custom gravity — per-body scale, and four source components that sum |
| **#640** | `BoxGravity` — a cube planet, each face along its own normal |
| **#642** | Gravity no longer keeps every body awake (0.137 → 0.042 ms/step, 300 bodies) |
| **#643** | The Console stopped redoing the whole log every frame (0.206 ms → 0.029 µs) |
| **#120** | Closed unbuilt: `stabby` + C ABI replaced by plain Rust `dylib`, no translation layer |
| **PR #651/#652** | The project's code loads into the editor as a `dylib` — its components appear in Add Component and render in the Inspector. Old projects migrate on open. Reload itself is still missing (#648) |
| **PR #653** | The monoliths, split. One file over 600 lines left, and it is named below |
| **PR #654** | The editor protocol left TCP for a local socket — closes the browser vector and the orphan-on-a-fixed-port confusion (#647) |
| **#655** | A `Joint` can be authored. Bodies are named with `Option<EntityRef>`, the Inspector has an entity picker and a World-panel drop target, and the three error paths that turned one failure into a frozen session stopped lying |
| **PR #660/#662** | The Console copies to the system clipboard, and filters by severity with one toggle per level instead of a minimum-level dropdown |
| **PR #664** | egui 0.35. Widget ids stabilised (see #641 below), numeric fields evaluate arithmetic — `9/2` is 4.5 — the name field keeps its caret, and a remotely spawned entity carries `Name` and `Transform` like a locally spawned one |
| **#656 / PR #667** | The editor and its project stopped burning two cores to show a still image. 200% → 3% CPU, 51.8 → 31.5 W. Four causes, one of them a bare `loop {}` in the headless runner that predated the issue |

---

## Backlog — performance and the files that hide it

⚠️ **This section predates the 13.9 ms budget and is host-side.** Every
number in it was taken on the author's desktop, measuring the *editor*.
That work stands, and none of it answers the target at the top of this
file — a handheld at 10 W is a different machine with a different
bottleneck. Re-measure before reusing any figure here.

Two sessions running, the thing that actually went wrong was not a missing feature. It was
work done per frame that did not need doing, in files too big for anyone to notice. Both are
the same problem seen from two sides, so they are one push.

### 1. Per-frame work that should not exist

Ordered by measured or estimated cost. **Measure first, then fix** — the last two wins came
from a number, and the two guesses before them were wrong.

0. **#656 — DONE.** Two cores, idle, to display a still image: **200% → 3% CPU, 51.8 → 31.5 W**
   on the same host. It was four things, not one. The windowed loop asked for the next redraw
   unconditionally and now derives `ControlFlow` from a `FrameRequest` each frame fills in,
   with the editor taking its answer from egui's `repaint_delay`. The *project* was never in
   that loop at all — `RemoteHostPlugins` has no window on purpose, so it ran under
   `default_runner`, a bare `loop {}` with no vsync; that was the 100% core, and it predated
   the issue. The Console and egui were feeding each other: egui's "changed id between passes"
   is itself a log line, so it landed in the Console, scrolled it, and produced the next one.
   And most window events change nothing drawn — 966 `AxisMotion` and 212 `Moved` against 486
   `CursorMoved` over five seconds of ordinary mouse movement.

   **It broke every "every N frames" clock, as predicted.** An idle editor draws about four
   frames a second, so the remote pull's thirty-frame cadence went from half a second to seven
   and a half. It is a `Duration` now, and **any new cadence must be too**.

1. **#645 / #691 — the remote pull. Measured, mostly fixed, one term left.** It was 32 ms a
   frame: 424.6 KB of JSON for 610 entities, 13.7 ms of it parsing, plus 7.5 ms rebuilding the
   mirror. Diffing server-side (#694) took the payload to **0.1 KB** and decode to 0.02 ms;
   skipping the mirror when the delta is empty (#695) took its 7.5 ms to **0.00**. Binary
   encoding was **cancelled, not deferred** — it would optimise 0.02 ms.

   What is left is `transport`: **4.4 ms**, waiting for the project to reach its next
   `Stage::First`. The HUD tooltip predicted this exactly — *"if this dominates, the fix is to
   stop doing it on the main thread"* — and it now dominates. That is #691 step 3.

   ⚠️ **`DenseScene` has no colliders.** With physics every entity changes every frame and the
   delta becomes the whole world again. **The diff solved authoring, not Play**, and nothing
   here has been measured with a solver running.
2. **#641 — egui `changed id between passes`. Mostly closed by PR #664, and the remainder is
   not ours.** The Console was the whole of the volume: widgets took automatic ids, handed out
   by order of creation, and `draw_message` emits a variable number of them — so one row
   renamed every row below it, and each of egui's complaints is itself a log line that shifts
   the rows again. Rows are now keyed on the log line. Measured: hundreds per session to zero
   in that panel.

   Two things to know before anyone reopens this. The check is `#[cfg(debug_assertions)]` and
   the red rectangles are drawn by that same function, so **none of it reaches a shippable
   build** — verified by running the editor in release. And the block that survived carries
   byte-identical ids across three builds, immune to every change: that is
   [egui #8343](https://github.com/emilk/egui/issues/8343), open upstream, where
   `with_layout(right_to_left)` inside `horizontal` warns spuriously. `menu_bar.rs` does
   exactly that. **This is no longer a performance item.**
3. **#666 — ⭐ THE NEXT ONE. The gather builds the whole world to draw twenty rows.**
   `Gather · entities` is **4.33 ms of a 13.5 ms frame** on 610 entities, and 96% of the gather
   stage. It is also the part nobody can make cheaper by closing a panel: collapsing every
   panel takes the UI pass from 9.2 ms to 3.1 ms and leaves gather untouched.

   The original framing — skip panels whose tab is not visible — cannot fix it: **the World
   panel is always visible**, so the entity walk runs in full, and the four sub-stages that
   visibility gating would remove total 0.1 ms.

   The cost is the shape of what is built. One `EntityDisplayInfo` per entity, each carrying a
   `Vec<ComponentDisplayInfo>`, each of those allocating a `String` for a short name that is
   `&'static str` at its source: **2440 String allocations a frame** to draw twenty rows. #695
   removed the reflected *values* from this path and got 0.9 ms of 5.26 — which is the proof
   the values were never the cost.

   A row needs a name, a depth, a child count, a component count and a scene. The full
   descriptor is read by the Inspector, for the selection. The gather is producing a structure
   shaped for the panel that shows one entity and handing it to the panel that shows six
   hundred.
4. **`asset_browser/tree.rs::render_root` rebuilds the whole folder tree every frame, twice**
   (Project and Engine roots), cloning a `PathBuf` per node. ~12 assets today, so invisible;
   the same shape as the Console bug that was not.
5. **Panels with unbounded lists are not virtualised. DONE** — the Console (#643) and now the
   hierarchy (#695): `ScrollArea::show_rows` draws the twenty rows that fit instead of all 610.
   UI pass 9.23 → 4.79 ms. What it demands is that a row's height be known before the rows
   above it are drawn, so `entity_row::row_height` is the single definition both the list and
   the row read, and **rows truncate rather than wrap** — a wrapped name would be taller than
   the list promised and every row below it would land in the wrong place.
6. **`kooch_gravity::plugin` walks and allocates its source list twice per frame** — once in
   `reconcile_world_gravity`, once in `apply_gravity_sources`. Small, but it is per frame.
7. **#569 — per-stage counters in the perf HUD. DONE (#695), and it should have been first.**
   The HUD's **CPU frame** section reports gather / UI / input / viewport / present / actions,
   what no stage claims, and the gizmo batch that runs outside the measured span; gather splits
   again into intern / entities / archetypes / types / assets. `Unaccounted` reads **0.01 ms**,
   so the split describes the frame rather than approximating it.

   Two things to know before reading it. **`cpu_frame_ms` does not include the remote pull** —
   `remote_sync_system` runs in `Stage::PreUpdate`, outside the measured span, so subtracting
   the pull from it subtracts something that was never inside. And **with vsync on the HUD does
   not move when real work is removed**: `KOOCH_PRESENT_MODE=novsync` exists so the frame can
   be measured at all. Vsync stays the default; an uncapped editor burns a GPU drawing frames
   nobody sees.

**The rule this session earned:** egui redraws everything every frame, so whatever a panel
does in its `draw` it does sixty times a second for as long as it is visible. The user's
report was *"it depends on how many panels are open"*, and that was exactly right.

**The rule the next session earned, which supersedes the order of this list:** four hypotheses
were raised about this frame in one day. The cull sizing (#689) was arithmetically damning —
815× of wasted threads, verified — and costs **0.076 ms**. Vsync was refuted outright. The
panels were real, and were found by asking the user to collapse them. The reflected values were
predicted at ~4 ms and delivered 0.9. **One hit in four by analysis; four in four by
measurement.** Item 7 was listed last and was the one that should have been done first.
Instrument, then argue.

### 2. The monolithic files — done, with one exception

Thirty files were over 400 lines. PR #653 split them. What is left over 600 is a single file,
and it was skipped on purpose:

| Lines | File | Why it is still there |
|---|---|---|
| 1148 | `kooch_editor_core/src/actions/remote_edit.rs` | Skipped as demolition-bound. **That premise is dead** — the remote stays (#647), so this file has to be split like the rest |

Twenty-five files sit between 400 and 600. That band is no longer an automatic split: the
threshold moved to 600, above which a file is *examined* for whether it went monolithic or is
carrying something that does not belong to it. Size alone is not the verdict. See `MEMORY.md`,
"Cómo se parte un archivo monolítico".

The full list, any time: `find crates src examples -name '*.rs' | xargs wc -l | sort -rn |
awk '$1 > 600'`.

### The audit that changed what "next" means — #669

**#669 — Roll a Ball, a first-user pass over the whole engine.** We have been testing the
engine as its authors; this tests it as its first user, in a project, against the public API
only. Each phase ends in a verdict about the engine rather than a feature for the game.

Reading the tree to plan it turned up how much of the presentation layer does not exist:

| Subsystem | State |
|---|---|
| Physics, gravity, meshlet path, materials | **strong** |
| Input, scripting | present, unproven from a project |
| **Lights** | **authorable and inert** — see below |
| Audio | kira backend, no `AudioSource` to author (#63) |
| Shadows (#476/#477), post (#254), particles (#97), runtime UI (#280/#96) | **missing** |

**`DirectionalLight`, `PointLight` and `SpotLight` exist in `kooch_ecs` and nothing reads
them.** `kooch_lighting/src/lib.rs` is nine lines; `kooch_render` never mentions them; #441 is
open. They are the exact shape of the gotcha in `MEMORY.md` — *a missing feature does not fail
the build: the component is authored, mirrored, draws a gizmo, and does nothing.* A user who
places a light and sees no change is the first bug #669 will find.

**#668 — how systems get to run in parallel**, given that users write their own. Blocked on a
scene that needs it: a hosting project currently does **0.17 ms** of work per frame, so there
is nothing to parallelise. #669's terrain phase produces that scene.

### #669 phase 0 is done, and what it found

A ball that rolls, a jump gated on a downward ray, and a virtual camera following it — built in
a project, against the public API only. **#671 phase 1 was proven for the first time**: the rig
had been written since 31 July and had never been seen to move a camera.

The finding that matters is not any single bug. It is that **neither Play showed a working
game, and each failed differently**: remote Play could not receive a key (#710, now closed by
#713), and the direct game lost the camera's target on load (#712). As the engine's authors,
both halves had green tests. Only using it from outside put them in the same room.

Seven separate cases turned up of **complete code with no reachable call site** — the input
crate, `feed_window_event`, the standalone Play path (#720), the dynamic type registry the
prefab inspector never asked, and three in the IDE launcher. **None of them fails a build.**
That is the argument for this epic, and it is no longer a hypothesis.

### Done — the action became data, and the player became parts

**#55 / #58 — closed.** `ActionMap<A>` was generic over a Rust *type*, which cannot be
serialised, inspected or edited, so the editor could never author a binding; it is deleted.

What replaced it is not a map. **An action is an asset** (`.inputaction`) with its own stable
id, and a component points at one **by guid**, picked in the Inspector like a mesh. Nothing in
gameplay names an action, so renaming one in the panel breaks nothing — the failure the
name-keyed version had by construction. Each action is enabled on its own, which a map could
not do, being all or nothing.

The panel authors all of it: five composites ported from Unity (2D/3D vector, 1D axis, one and
two modifiers), processors on a binding, on a composite head and on the action, reorderable
because order is meaning. Editing a file is picked up without a restart.

Three lessons went into the engine rather than into this document:

- **Asset types register themselves at link time** (`register_asset!`). The loader and its
  storage were two hand-written lists in two places, and `.inputmap` shipped with its loader in
  both and its storage in neither. Nothing central lists an asset type now, so nothing central
  can leave one out.
- **The editor's component list is checked by a test.** A `*ComponentsPlugin` missing from
  `EditorPlugin::build` made a component the menu offered and then refused; that was the fifth
  time. The test scans the workspace for them.
- **The editor says when the project library is older than its sources.** It loads the `.so`, it
  does not build it, so a component written and not compiled was simply absent with nothing to
  explain it.

**What is left of the map idea:** bulk enable/disable. A pause menu wants to silence a *set* of
actions at once, and per-action `enabled` makes that a loop. `ActionMap::priority` is still
written and never read — when the pause exists, that is the consumer that should shape it,
rather than guessing now.

**Alongside it, in the game: decompose `PlayerController`.** Four fields that are three
capabilities, together only because all three happened to belong to the player. It becomes a
`Player` tag, `GroundMovement` and `Jump` for how *this* body behaves, and an **intent**
written by whoever is driving.

Separating the intent from the input is what makes the systems reusable: a system that reads
keys serves only the player, while one that reads an intent serves an AI writing the same
thing. It is also what lets #55 land without touching gameplay.

> Working mode from 2026-08-02: **the engine's author writes this code.** Guidance, review and
> diagnosis rather than patches.

### Then, the features that were next before this

**#562** (scene queries), **#567** (PD/PID controllers), **#639** (split `RigidBody` into
`RigidBody`/`KinematicBody`/`StaticBody` — a scene-format migration).

> The standing rule: implement what Rapier offers, warn for what it does not. See `MEMORY.md`.

---

## Backlog — editor, because multi-scene is half-reachable

1. **#619 — no way to create a scene.** There is no New Scene. Multi-scene cannot be
   exercised without files already on disk.
2. **#613 — additive scenes over the wire.** The protocol assumes one scene, so additive
   loading is disabled while a project is open. The only place a merged feature is
   knowingly incomplete.
3. **#591** context menus, **#592** a Game panel separate from View.

---

## Prefabs — done, and what it taught

**#611** shipped in four PRs. Both questions it flagged got answered: a document instanced
as a unit needs exactly one root and says so rather than picking one, and identity is
remapped per instance so two copies never claim to be the same entity.

What is worth carrying forward is the reversal. Phase B first stored an instance's entities
in full *and* a link, on the argument that a scene should open with its prefab missing and
that nothing should depend on load order. Both true, and outweighed:

> **Every prefab bug found while building it was the same bug.** A value held in two places
> drifts.

The Inspector showed a prefab from before it was overwritten. A component removed from an
instance came back on the next save. A second scene never saw a change. The project kept
instancing from the copy it read first. Four fixes, one class, still producing new ones.

A scene now stores the reference and the overrides; the values exist once. The load-order
dependency came back and is accepted, because an unresolvable prefab spawns
`missing prefab [guid]` — a broken reference is something the scene *shows*, where a stale
copy looks exactly like a correct one.

**The rule this leaves:** when a design keeps producing bugs that each need their own fix,
count how many are the same shape before writing the fifth.

### The one limit left

A child entity **added** to a prefab appears when a scene loads, but a running editor does
not grow one mid-session — placing it means positioning relative to whatever the instance
became. Worth its own issue if it ever hurts.

---


## Animation — decided, not started

Three layers people routinely merge, and merging them is how one cannot change without breaking
the others:

| Layer | Issue |
|---|---|
| Deformation — bones to vertices, on the GPU | **#453** |
| Pose source — clips, sampling, blending | **#92** |
| Control — which pose plays and how it mixes | **#717** |

**#717 is the one that governs.** One Playables-style graph under skeletal animation, scene
animation and a timeline, where **a timeline is a node** and so is a state machine, so they nest
in each other. Unity does not get this far: Timeline and Animator are both Playables but
nesting one in the other shows the seam. Here nodes are flat arrays with `u32` indices, so
nesting is writing a number — and **the authored asset is just the arrays' initial state**,
which means authoring and runtime composition are the same operation.

**#715 — scene animation.** Any reflected field on any entity: *if you can see it in the
Inspector, you can animate it*. Three kinds of track, and the middle one is the interesting
one — **whether a component is present is sampled state, not a fired event**, so scrubbing
backwards undoes it. Unity cannot do that, because adding a component there runs `Awake`.

**#716 — target identity by hashed name path**, shared by both. Stable across reloads and
re-instantiation precisely because it derives from nothing assigned at runtime.

**#92 — adopt, do not write.** `ozz-animation-rs` is a deterministic Rust rewrite of ozz, data
-oriented, engine-agnostic. It brings sampling, blending and the skeleton; the graph is ours.

**Motion matching is a later feature, gated on data rather than effort.** Published work uses
4.6 h (LAFAN) to 44 h (100STYLE) of capture. With a handful of clips it produces a *worse*
result than a blend tree, and the failure mode is concluding the technique is bad when the
dataset was. Public corpora are frequently non-commercial — check before building on one.

## Larger, not yet started

- **#566 — world cells.** Scenes become streamable content; entities transit between cells.
  Scene and cell are orthogonal axes. This is the piece planet-scale actually needs.
- **#614 — terrain LOD.** Research, with an honest verdict: dual contouring beats
  marching-cubes-plus-Transvoxel on an octree, but feeding octree nodes through the meshlet
  pipeline is **an unproven hypothesis**. Nobody found doing it. Measure the cost of
  clusterising one dirty node before any design commits.
- **Rendering backlog** — #441 PBR, #476/#477 shadows, #450 GI, #250 sky materials, #485
  clustered light culling, #484 HDR, #481 motion vectors and FSR. Technically unblocked, but
  **sequenced behind #392 on purpose**: they are the passes whose arrival decides whether the
  render graph is worth keeping, and adding them beside it forecloses the question.
- **#558** — shippable builds must exclude the editor. Security, not size.

---

## Deliberately not scheduled

- **Adopting Bevy wholesale.** Would save roughly two thirds of the codebase and cost the
  parts that are the point: the GPU-driven meshlet renderer, planet-scale streaming, and the
  editor — none of which Bevy provides. Settled in #605.
- **Making a dynamic body follow its parent.** No engine supports it; Godot has failed to
  for years. The supported answers are compound colliders (#615, done) and joints (#560).
