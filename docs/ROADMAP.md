# Roadmap

What is next and why, ordered by what blocks what. Issue bodies hold the detail; this is the
map.

Companion to [`MEMORY.md`](MEMORY.md), which records decisions already made. If the two
disagree, `MEMORY.md` wins on *decisions* and this file wins on *order*.

**There is exactly one "Next" heading.** Everything else is `Backlog` or `Done`. Three sections
called Next is how a roadmap stops being read.

Last updated 2026-09-01 — 🔴 **the standing queue was mostly already done**: of the five issues in the 2026-08-30 order, three closed by reading the tree, and two of those had been fixed with nobody recording it. Verify against the code before scheduling. **Next is #719** — a scene naming an unknown component type loads silently, which already cost a shipped build every `Spin` pivot. — 🔴 **#1009 landed: the lamps cast again.** Four fixed-size caps deleted them in silence — a dispatch past 65 535 workgroups, a 16 384 pair list, a per-view clear over per-frame counts, and a 256-caster moved list that voided the page cache every frame. Each failed as a resident, correctly keyed, EMPTY page, which reads as lit. The distant tier is derived from `page_level` rather than tuned. **Next is the OneXFly measurement** — 5.36 ms on a 9070 XT decides nothing. — 🔴 **the bright patch is fixed: it was Olsson's receiver bound, and the debug view had no colour for it.** A page drawn *without one caster* paints green, the same as a bad bias, so four correct eliminations pointed nowhere before the bound was tried. #1022 landed and is worth its claim — `geometry walk 245` against `pair tests 452 432`. **Next is #1009**, the distant-light tier: 32 point lights exhaust the pool (`slice used 1024/1024`, `free 0`) because a lamp is either a full chain or nothing, and Unreal's middle tier — one page per distant light, round-robin at one update a frame — is what is missing. — 🔴 **there is no render distance in this engine.** `perspective_infinite_reverse_rh` never receives `PerspectiveCamera::far`, and no `draw_distance`/`cull_distance` exists anywhere in the tree, so every instance in a scene is in frustum forever — free at 190 m, the whole frame at 1410 m. Two silent limits cost a day each (#996 buffer sizes, #997 the 65 535 dispatch ceiling) and `dense.scene` opened; what it showed is in the order below. 🔴 **a frame cap is a PERFORMANCE setting on this part: the same work costs 3.9 ms of GPU capped at 72 fps and 13.2 ms uncapped, because capped the GPU idles 68 % of the time and holds ~1210 MHz instead of throttling to ~850.** `gpu_busy_percent` reads 32 % and the scopes agree. **The budget is met with the preset below** — 13.88 ms frame, and at 8 W capped only **28 % of it is used**. The lever was the upscaler, not the shading: `upscale: 3` (FSR 3.1) cost 11.355 ms of a 23.36 ms frame and `upscale: 2` (SGSR 2) costs 2.062. 🔴 **The ~11 ms shading floor #885 was built to decompose no longer exists** — the whole shading pass is 3.272 ms today — and the instrument built for it measured something else instead: a full-screen sweep per material **in the project** costs 178 µs on the device, which is 0.71 ms here and 3.7 ms for a game with twenty materials. **#826 is removed, not deferred.** Cutting a froxel's light list by COUNT is incompatible with a cluster grid being continuous, and that is a property of the idea rather than of any implementation of it: see the entry below. The remaining queue is the contact march's cap (#839) and #731. The budget is unchanged, still unmet, and now measured against the SETTLED clock rather than the boosted one — 40.7 ms, not 27.8.

---

## The constraint everything is now measured against

**72 FPS at 10 W TDP on the OneXFly F1 Pro.** The bar a game made with
this engine has to clear.

🎉 **It cleared it on 2026-08-25**, on `many_lights` — 100 point lights,
the harshest scene this project has — at exactly 10 W. Frame 13.88 ms
(the 72 Hz cap, held flat across the whole capture), GPU 12.2 ms, so the
frame is limited by the compositor rather than by the GPU for the first
time. It got there from **11.0 FPS** in one day: see #954.

⚠️ That is one scene on one device, and the scene's lights were static.
The same scene with its lights in motion is the case the content cache
cannot help, and it has not been measured on the device yet. Clearing the
bar once is not the same as owning it.

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
target. Every number here is taken on the device.

### 🔴 …but not every number here was taken at 10 W. Warm the device first

**Measured 2026-08-16, `many_lights.scene`, five unbroken minutes from
the game's first frame:**

| | first 2 min | after | |
|---|---|---|---|
| frame | **27.8 ms** | **40.7 ms** | +46 % |
| GPU work | 25.1 ms | 31.3 ms | +25 % |
| `sclk` | ~1150 MHz | ~850 MHz | −26 % |
| package power | **12.0 W** | **10.0 W** | |

The handheld runs a boost budget for about two minutes and then settles.
RSS and VRAM are flat across the whole run, so nothing in the engine is
accumulating — the GPU is doing the same work at a lower clock.

**The settled state is the one this budget is about**, since the target
is written at 10 W. So the frame to judge against 13.9 ms is **40.7 ms,
2.9× over** — not the 27.8 ms a short capture reports.

🔴 **Any capture shorter than ~2 minutes measured the boosted chip.**
That includes most of the numbers in the sections below, and it retires
a discrepancy that was being treated as a mystery: a capture reading
41.4 ms and another reading 27.8 ms on the same build were the same
engine in two thermal states, not two engines.

⚠️ **Procedure, from now on:** let the game run two minutes before
capturing, or capture long enough to contain the transition and read it
with `read_capture --over-time`, which prints the drift and says so.

🔴 **…and this whole subsection describes an UNCAPPED machine.** With the
frame rate capped in hardware there is no settling and no drift, because
the GPU never runs long enough to reach its power cap — see the next
section. A capture has to record which of the two it was taken on, or the
two are averaged into a machine that does not exist.

### 🔴 A frame cap is a PERFORMANCE setting on this part, 2026-08-20

⚠️ **Read the resolution column before comparing anything below.** The
first captures of the day were taken at 1920x1080 and the last two at a
forced 1280x720. At `render_scale: 50` and `shading_rate: 2` that is
480x270 against 320x180 — **44 % of the samples** — so a row from one
group cannot be subtracted from a row of the other. One table published
here on 2026-08-20 did exactly that and is retracted below.

**The comparison that holds, because both sides are 1280x720:**

| | 4 W, uncapped | **8 W, capped at 72** |
|---|---|---|
| frame median | 13.96 | **13.88** |
| p99 / max | 21.60 / **47.09** | **14.46 / 15.25** |
| **GPU** | **13.2** | **3.9** |
| `frame/GPU` | 1.06 | **3.5** |
| `shade: compute (half rate)` | 3.858 | **1.117** |
| `sgsr2` | 2.606 | **0.722** |
| `shadows` | 1.251 | **0.374** |

**The same work at the same resolution costs 3.4× less GPU time**, and
every pass moves by the same factor — which an engine does not do. The
clock does:

```
sclk, capped at 72 fps   ~1210 MHz   (12 samples over SSH, during the capture)
```

Capped, the GPU is idle **68 %** of the time, so it neither heats nor
reaches its power cap and holds its boost clock. Uncapped it throttles
and every pass takes three times longer. Rendering 144 frames to display
72 pays for the same work three times.

🟢 **Two independent instruments agree.** `gpu_busy_percent` reads
**32 %** off the kernel; the GPU scopes say 3.9 ms of a 13.88 ms frame,
which is 28 %. The four points between them are the compositor, which our
scopes cannot see and never claimed to.

🔴 **What this pair does NOT separate: the TDP doubled too.** 4 W
uncapped against 8 W capped changes two things at once, so "3.4×" is the
two together. The `gpu_busy` and `sclk` readings argue the cap is doing
most of it — a part idle 68 % of the time is not power-limited — but the
run that would settle it has not been taken: **8 W uncapped at 1280x720**.

⚠️ **It splits a rule this file states as universal.** *"Warm the
handheld two minutes, it gets 46 % slower"* describes an **uncapped**
machine. Capped there is no drift at all: 13.89 -> 13.88 ms across 60 s,
with the GPU moving 3.89 -> 3.99 (+2.6 %). Two machines, and a capture
has to record which one — and now at which resolution.

The cap also fixes the pacing: max frame **15.25 ms** against 47.09.

🎯 **Against the budget, at 1280x720: 3.9 ms of GPU in a 13.9 ms frame is
28 % used.**

### ❌ Retracted the same day: "at low TDP the frame stops being the shading pass"

A table published here compared the 4 W capture against the 10 W one per
pass and concluded the short passes nearly double while the shading pass
grows 18 %. **The 4 W capture is 1280x720 and the 10 W one is 1920x1080**,
so the two differ by 2.25× in samples before TDP is considered.

It does not renormalise, either: `shadows` is sized by
`shadow_cascade_texels` and does not scale with the screen at all, and
`sky` is a raymarch whose cost depends on how much sky the camera sees.
The conclusion may still be true — it is simply not measured. **The pair
that would measure it is one TDP at one resolution.**

### 🟢 The configuration that meets it, measured 2026-08-20

**13.92 ms median, 4653 frames, zero drift, at 1920x1080 and 10 W
settled.** The budget, on the device. This is the baseline a handheld game
ships with unless it has a measurement saying otherwise.

| `.rendersettings` | value | why |
|---|---|---|
| `compute_shading` | `true` | the tiled path — half rate and the reduced-rate upsample both require it |
| `shading_rate` | `2` — *Half, one sample per 2x2 quad* | a quarter of the shading threads (#825) |
| `upscale` | `2` — *SGSR 2* | **2.062 ms against FSR 3.1's 11.355**, same scene, same session |
| `render_scale` | `50` — *Performance, 50 % (2x)* | the shading pass costs what it costs per PIXEL |

```
frame 13.92 ms (median)  ·  GPU 9.7  ·  budget 13.9
raster + shade                 7.862  [self 0.954]
├─ shade: compute (half rate)  3.272
├─ sgsr2                       2.062
├─ tonemap                     0.566
├─ motion vectors              0.553
└─ shade: upsample             0.455
shadows 0.549 · blit 0.424 · sky 0.415 · cluster grid 0.136 · cull 0.035
```

🔴 **The largest single item is the upscaler, and choosing the wrong one
costs more than every shading optimisation put together.** Same scene,
same build, same session, with `upscale: 3`:

| | FSR 3.1 | SGSR 2 |
|---|---|---|
| the upscaler | **11.355 ms** | **2.062** |
| whole frame | **23.36 ms** | **13.92** |

FSR 3.1 is neither broken nor a regression — it is a desktop technique,
and its transliteration measured 11.682 ms here when it landed (#884).
What was missing was a project whose asset picked the other one.

⚠️ **13.92 ms is a vsync step, not a cost.** p99 is 25.64 and max 29.07,
almost exactly double — a missed vblank rather than work. The panel is
144 Hz and the frame is sitting on half of it. What the frame costs is
the **GPU's 9.7 ms**, with `frame/GPU` at 1.45. There is room under the
number, not on top of it.

**Dropping the output resolution buys more than any of these**, because
`render_scale` is a percentage *of the output*: a smaller window shrinks
the render target with it **and** everything `render_scale` does not
touch — the resolve's output, the tonemap, the blit. Observed on the
device on 2026-08-20; not yet measured here, and it should be.

🔴 **A recommendation, not a default.** `RenderSettings::default()` stays
what the engine did before any of this existed, for the reason
`settings.rs:510` argues at length: a serde default is not a
recommendation, it is what an old file silently becomes. A project that
wants these values sets them.

---

## 🎯 The order, decided 2026-09-01 — the queue was mostly already done

Five issues stood in the 2026-08-30 order. **Three were closed by reading the
tree rather than the issue**, and two of the three had been fixed without anyone
recording it.

| | was | is |
|---|---|---|
| #1011 | group arena sized by the cull rectangle | **already fixed** — `frame/pages.rs` passes `scene_params.group_capacity` |
| #1006 | moved-caster cap of 256 | **fixed today** by `a4a95c9e` |
| #1018 | a camera translation redraws 1472 pages | **same cause as #1006.** The pool counters said `0 popped · 0 evicted · 100 % hit` and were true and irrelevant — the generation ABOVE the pool was voiding every page, and no pool counter can see that |
| #1012 | play mode pulls the whole world | **open.** `moved_cache.rs:89` still reads `full: appeared || stale` |
| #1019 | the far-layer page needs a two-frame guard | **open.** Invalidation is per LAMP, so both pages come back together and the test never reaches the state the corruption survives in |

🔴 **Three issue bodies described a tree that had moved.** That is now the
default assumption: verify against the code before scheduling anything.

### What else closed

**#1001** — a second directional light corrupts the paged sun — closed *not
planned*. `Casters.sun` is a `bool`, `sun_gens` hashes one direction,
`inti_pages.sun` is one `vec4`; two suns get one clipmap stamped by whichever
was extracted last. Guarding a constraint **#782** exists to remove has a short
life, so the finding moved into #782 instead.

**#839 item 2** — the contact march has a tap budget. The cost is
`steps x lights` and only the first factor had a number. `dominant_only` bounds
the second to one and is **off by default** since the settings moved to what the
project renders with. `TAP_BUDGET = 32`. Items 1 (the device measurement) and 3
(the env switch, which already existed) are not this.

### The extension point, re-scoped

**#392** and **#872** are mechanism and discovery, and neither is worth much
alone. Audited:

- The plugin `Stage` enum already spans `Startup` → `Last`. **Code at any stage
  is already reachable** for CPU systems.
- `PluginSystem` gets `&mut dyn Engine` — spawn, despawn, register, add_system,
  log, set_data, get_data. **No device, no queue, no encoder.**
- `GpuSystem::dispatch(&self, pass: &mut wgpu::ComputePass)` — a compute pass.
  So **a compute shader is expressible and a post-process is not**.
- `add_system` appends. Two plugins that both want to run late fight over load
  order instead of declaring a constraint.

#392 is the door six written issues wait behind — #254, #33, #116, #484, plus
#250/#784/#70 for shader authoring. It is not an isolated refactor.

⚠️ It moves the pass order this week stabilised: the page marking runs BETWEEN
`render_geometry` and `render_shading` so it reads this frame's depth. As
systems that becomes an explicit `after`; a conversion that drops it gives
shadows one frame late with nothing on screen to say why.

### Next — #719, and the reason is this week's evidence

**A scene naming an unknown component type loads silently.** Grepped: there is
no `warn` anywhere in the scene loader for a type that does not resolve.

It already cost a shipped build. `Spin` was behind the `testing` feature, so
exported games opened with **every pivot gone** — lights that move in the editor
and stand still in the game, no error, no log. Shipping `Spin` fixed the
symptom; this is the class.

The engine's stated policy is **do not write migrations, break the data and fix
it by hand**. That is right for one private project — and it is only safe if
breaking is LOUD. It is silent.

Behind it: **#684** (`Cargo.lock` is gitignored, so builds are not
reproducible — for an engine that vendors itself into every project and compiles
on a handheld) and **#686** (`--no-default-features` does not compile; measured,
two errors, `default_asset_plugin` cfg'd out).

### Measurement, deliberately dropped

The OneXFly capture gated #839's item 1, #865 and the classic-path decision. The
user's call on 2026-09-01: *"dejemos las mediciones, están bastante bien."* The
gate is lifted; #865 stays unmeasured on purpose.

## 🎯 2026-08-31, evening — the lamps never cast, and four caps deleted them in silence

**#1009 landed.** `lamp survivors` went 0 → 33 851 and `dense.scene` has lamp shadows.

Four independent bugs, one signature: the page is resident, correctly keyed, and
**empty**. A cleared page is far depth under reversed-Z, so every reader answers
"nothing occludes" — the lamp casts nothing and every counter reads healthy.

- **The cull dispatch passed one workgroup dimension.** `pairs * meshlets_per_mesh`,
  where the factor is the SCENE-WIDE meshlet max: 1.17 M workgroups against 65 535.
  An indirect dispatch past the limit is undefined; it did nothing, so the lamps'
  two heavy passes never ran. `limits.rs` had already written the warning.
- **The pair list was a constant 16 384.** Overflow counted into a word nothing read.
- **The per-view clear spanned the lamps' buckets.** `record` runs per view, the cull
  per frame, so the second camera wiped what it would not refill. The comment in
  `LampCull::record` already claimed the opposite.
- **`MOVED_CAPACITY` was 256 against 2026 spinning casters.** Not a memory budget — a
  cache switch. 16 bytes a sphere against a 52 MiB atlas.

**The distant tier is derived, not tuned.** `page_min_pixels` was a cliff that silenced
34 lamps while the pool stayed at `1024 / 1024`. A light now drops to the top of its
chain — one page per face. The test is Epic's, `MinMipLevel == MaxMipLevels - 1`, which
`page_level` already answers, so no threshold is needed.

**The lesson is the funnels.** A lamp's geometry passes four fixed-size caps here, and
each one fails as a lit shadow. Unreal have none: `OverlapsAnyValidPage` culls the
instance against the page table itself, one Nanite pass for every shadow view at once.
That is the shape to move to — it removes the class, not the bug.

### Also
`Spin` ships with the engine; exported games had lights that moved in the editor and
stood still in the build. A new `.rendersettings` opens at the tuned values. Four
standing test failures fixed, all in the classic path.

### Next
- **Measure on the OneXFly.** 5.36 ms on an RX 9070 XT says nothing about 13.9 ms there.
  Every other decision waits on this, including whether the classic path survives.
- **`chunks_for` for the lamps' cull.** `pairs * scene max` is legal now, not small.
- **Geometry-first expansion for lamps.** `pair tests 303 015` for `3 020` pairs — 1 %.

## 🎯 2026-08-31 — the sixth cause was none of the five, and the instrument could not name it

The bright patch inside a shadow is **fixed**. It was **Olsson's receiver bound**
(#940/#949), and it is removed rather than repaired.

The bound rejected a caster whose nearest point lay beyond a page's furthest
RECORDED receiver. That record only ever covers the receivers that marked *that
level*, while the reader **climbs to coarser levels** whenever its own does not
answer. A receiver that climbed met a bound written by other receivers entirely
and lost the caster it needed — the page drawn with the ground in it and without
the occluder, which shades lit.

Removed, not fixed, and the arithmetic is why: it saved **7 % of the sun's
candidates**, measured (`rejected 18 sun` against 227 emitted pairs). Making it
correct means the marking writing the bound on every level the reader could
reach — seventeen atomics per sample instead of one, over 1.2 M samples a frame.
`PAGE_LOD` moved into the word it vacated, so the reader's new jump table costs
no memory and `PAGE_CELL` stays at six.

### 🔴 The lesson is the instrument, not the bug

`VirtualPages` separates three faults: **red** no page, **yellow** allocated and
never drawn, **green** the comparison is wrong. Its yellow tests `stored <= 0.0`
at a **single texel**, so it only catches a page nothing was drawn into. **A page
drawn without one caster is green** — indistinguishable from a bad bias.

The hunt therefore went through the bias, the depth space, the page latency and
the pass order before the bound was tried, and *each of those was ruled out with
evidence that pointed nowhere*. An instrument that cannot name the fault it was
built for is worse than a slow search: it makes the wrong answers look
eliminated. Giving that case a colour is the first thing to do next time this
class of artefact appears.

### What the search left behind, all of it real

Each of these was a genuine defect found while looking somewhere else, and each
is in `development`:

- **The sun's cull box sat on the camera**, while `sun_window` sits on the
  snapped page grid. Same size, offset by however far the camera is into its own
  page — the window's lowest band, up to a whole page and **655 m at the coarsest
  level**, lay outside the box the cull runs against.
- **The page table was built after the culls**, so a cull could not ask about
  pages and had to gate on a box decided apart from the marking. Now: lamp cull,
  invalidate, compact, pyramid, level culls, expand.
- **The marking read the previous frame's depth.** `Vbuf64Stage::render` is split
  into `render_geometry` and `render_shading`; the page work runs between them.
  Unreal's order, and the only window where both halves are right at once.
- **A resident page with no content read as a hit**, which ended the reader's
  climb at the one level that could not answer.
- **A PCF tap that left its page was clamped to the edge**, so a shadow crossing
  a page seam read the receiver's own page and found nothing.
- **Page dilation** (Epic's `PageDilationOffset`), with the diagonal dithered off
  the thread index — a fixed direction asks for two of the eight neighbours and
  never the side the camera is moving towards.

### #1022 landed, and it is worth what it claimed

```
geometry walk      245 · 1 per pair
pair tests     452 432 · 1993 per pair
meshlet pairs      227
```

The descent touches 245 pages to emit 227 pairs — 93 % hit rate, **~1850x under**
what pairing spends for the same pairs. `shadow_page_geometry`, off by default,
and `both_expansions_emit_the_same_pairs` compares the two lists as sets.

Also landed: the reader **jumps** to the level that answers instead of walking up
to seventeen misses per pixel per light (Unreal's `LODOffset`), and
`the_page_passes_are_profiled` now counts GPU scopes opened against closed — a
reorder dropped a `close`, wgpu rejected the encoder every frame, and 509 green
tests said nothing while the compiler reported it as an unused variable.

### Next — #1009, the distant-light tier

Point and spot lights still cast nothing in `dense.scene`, and the reason is
measured: **the pool is exhausted**. `slice used 1024/1024`, `free 0`,
`bump 1024 of 1024`, `preempted 4`, and the pressure system already
`asking coarser — locals +2`. A page with no slot is not readable, so the shadow
is absent while every other counter reads healthy.

A lamp here is binary — a full chain of six faces, or gated to nothing by
`page_min_pixels`. Unreal have the middle: **single-page shadow maps** for lights
under a footprint threshold (`r.Shadow.Virtual.DistantLightMode`), updated
**round-robin at one light per frame** (`MaxDistantUpdatePerFrame`), allocated
from their own id range below the full ones. One page per light instead of a
chain.

That tier is #1009, rewritten with the plan and labelled `next-session`.

---

## 🎯 2026-08-30, evening — five causes eliminated, the sixth still standing

Eight hours on one artefact: a bright patch inside a shadow on
`dense.scene`. It is **still there**. What the day produced instead is
four measured wins and, more usefully, a list of what the artefact is
NOT — each entry closed by a capture rather than an argument.

### What landed (#1023)

| Change | Effect |
|---|---|
| Receiver-plane bias | `shadow_normal_bias` **8.0 → 1.0** with no acne returning |
| A cleared lamp page outlives its generation | **902 → 8** pages rasterised per frame |
| The panel is a table with alerts | What made the last four rounds diagnosable |
| `shadow_density` past 100 % | Epic's LOD bias; the pass already clamped to 400 |

Off by default and unfinished: the **march** (`shadow_page_march`,
Unreal's SMRT shape, +0.67 ms, introduces peter-panning) and the
**residency pyramid** with its overlap query (#1022 units 1 and 2,
nothing reads them).

### The five, and how each was closed

1. **Resolution** — 16x the shadow density did not move it.
2. **A scalar bias** — 0, 8, and a constant raised to the same step were
   indistinguishable at 79° of incidence. No magnitude substitutes for a
   direction; Unreal's is a two-component gradient.
3. **The marking running on last frame's depth** — it persists with the
   camera perfectly still, so it is not the phase order.
4. **Missing or unfilled pages** — the debug view paints it GREEN, and
   `unfilled_sun` reads 0 in every capture.
5. **The single-tap query** — the march does not fix it and breaks cube
   shadow edges.

### Where it goes next

The reading the elimination now supports, and the user's from the
start: **the meshlet cull and the order the shadow work happens in.**

Unreal walk INSTANCES, project their bounds into the shadow map, and
ask whether any page they touch is resident — work generated from the
geometry side. This engine goes the other way: a per-light cull produces
survivors and the expansion pairs `pages × survivors`, so the two halves
are decided independently and can disagree. That inversion is **#1022**,
and it is the last structural difference from Unreal not yet ruled out.

⚠️ Its unit 3 is not justified by today's numbers on their own —
`unfilled_sun` is 0 and pairing beats scatter 4x — so it has to be
entered as a CORRECTNESS change, not a performance one.

### Method, three notes worth keeping

- **The instrument came before the hypothesis, except when it did not.**
  Every real finding came from a number on the panel; every wasted hour
  came from a theory tested by rebuilding instead of by reading one.
- **An instrument can be wrong too.** A debug view painted the whole
  frame orange because it recomputed "the level this pixel wanted" from
  the READER's containment floor, while the marking picks
  `max(containment, density)`. Two nouns, one name.
- **Falsify the test, not just the code.** Three of this branch's tests
  were deliberately broken to prove they bite. One of them — the
  overlap query's mip choice — caught a closed-form bit trick that was
  wrong in the expensive direction on the first try.

## 🎯 The order, decided 2026-08-30 — the cost was never where it looked

A night on `dense.scene` took the GPU frame from **10.88 ms to 3.5 ms**
and the editor's frame, with a key held, from **49 ms to clean**. Almost
none of it was the thing the profiler pointed at, and that is the lesson
worth keeping.

### What actually paid

| | before | after | what it really was |
|---|---|---|---|
| `page cull` | 7.40 ms | **0.80** | 2.3 GiB of `clear_buffer` a frame, not culling |
| `other`, key held | 41.23 ms | **~1** | one listener thread, not payload size |
| the standalone build | blue screen | renders | 10112 texels against an 8192 limit |
| pages, one view | 4096 | **6144** | the cap belonged on the layer |

**#1002** put the camera's cull on instances instead of an
`instances x heaviest mesh` rectangle, and the clipmap kept the
rectangle: seventeen culls a frame, and toggling the setting moved the
number by 0.02 ms because it never reached that path.

But the 4.5 ms that survived were not work. `group_max_err` is indexed
by LOD group — 24 108 of them — and the page path handed it the cull
rectangle, asking for 16.7 M. `reject_reasons` is only read by an
overlay wired to the camera's cull. **Both were cleared in full on all
seventeen dispatches.**

### 🔴 Three failure modes this roadmap now watches for

**1. An instrument that cannot express the complaint.** Two rounds of
"it drops frames when I move" were reported against a HUD whose every
row is a mean over sixty frames — the shape built to hide a hitch. A
30 ms spike inside a second of 6 ms frames moves the average by 0.4 ms.
Adding `worst:` made a 113 ms spike visible **with the GPU at 3.55 ms**,
which ended the search in one reading.

**2. A constant that meant a duration.** `DEFAULT_MAX_AGE = 60`, and its
own doc called it "a second at 60 Hz". At 150 FPS it became 0.4 s, so
the page cache forgot twice as fast — *a stutter that arrived with the
optimisation that caused it*. Anything measured in frames that means a
duration is a bug waiting for the frame rate to change.

**3. Fixing the half that does not cost anything.** `notify` made the
CLIENT stop waiting for a reply nobody read. The server's single
listener kept blocking on the main loop before it could accept the next
connection, so holding a key put two blocking connections through a
queue that serves one. **The cost moved rather than left.**

### What is open, in order

1. **#1012** — a spawn makes the cheap play-mode pull give up and fall
   back to reflecting every field of every component of every entity.
   Send the entity that appeared, not the 2159.
2. **#1018** — a camera translation redraws 1472 pages in one frame,
   between two frames that redraw none. ⚠️ The issue carries an explicit
   *do not theorise before the counter exists*: three hypotheses have
   already been wrong on this artifact.
3. **#1017** — a page is square to the SUN and a rectangle on the
   ground, stretched by `1 / cos(incidence)`. At 10° that is 5.8x, and
   it is why raising `shadow_bias_max` cannot fix the contact gap: the
   bias multiplies one texel, so it overshoots on the short axis while
   still falling short on the long one.
4. **#1011** — half done. `visible_meshlets` is still sized by the cull
   rectangle, 16.7 M entries a level.
5. **#1019** — the far-layer guard. `a_page_on_the_far_layer_draws_the_same`
   passes with the layer gate disabled, so #1020 is validated by
   observation and not by that test.

### The method that worked, and the one that did not

Every real finding this session came from **reading a number**, and
every wasted hour came from **asserting one**. The worst was computing
`10112` correctly and discarding it against a `max_texture_dimension_2d`
of 16384 that was never read — the engine asks for 8192. A calculation
checked against an invented constant is worse than no calculation: it
closes the right line of enquiry with false confidence.

---

## 🎯 The order, decided 2026-08-28 — the dense scene opened, and named three things

`dense.scene` — 2024 instances over 1410 m, 64 orbiting lights, all
casting — is the first scene in this project that is neither a room nor
a bench. Getting it to open took two fixes, and what it showed once open
is worth more than either.

### What it took to open it at all

- **#996** — `wgpu::Limits` fields not named fall through to
  `Limits::default()` **without a word in the log**. The lamp cull's
  buffer was sized by a default nobody wrote.
- **#997** — `dispatch_workgroups(x, …)` caps at **65 535** per
  dimension on every backend. 2024 instances × 4953 meshlets ÷ 64 is
  156 639, so the frame died on a validation error and kept dying.
  Folded into two dimensions; below the ceiling the dispatch is
  bit-for-bit the old one.

Two instances of the same shape in two days: **a limit inherited in
silence**. That joins "a feature that exists and is not reached" as a
failure mode this roadmap now watches for by name.

### 1. 🔴 There is no render distance. At all

`projection.rs:92` builds `perspective_infinite_reverse_rh`, and
`PerspectiveCamera::far` is **never passed to it** — `far` reaches the
shadow cascade fit (`shadow/pass.rs:144`) and the page grid
(`shadow/pages.rs:544`) and nothing else. `grep` finds no
`draw_distance`, no `cull_distance` and no `render_distance` anywhere in
the tree.

So every instance in a scene is inside the frustum forever. At 190 m
across that is free. At 1410 m it is the frame, and the owner named it
from the seat — *"empieza a pegar en la performance"* — before any
number below was in.

⚠️ **The infinite projection is not the thing to change.** It is what
makes `ndc.z = near / distance` exact, and contact shadows, SSR, fog,
the atmosphere and the temporal upscaler all read depth that way;
`projection.rs` already argues this at length. What is missing is a
**cull-side** reach — a distance past which an instance is rejected
before it ever becomes meshlets, plus a fade so it does not pop.

It wants the same instance-level pre-pass as item 2, and it is adjacent
to #448 (imposters), which is what belongs on the far side of a reach,
and to #950, which would cut many of the same instances for a different
reason. **No issue yet.**

### 2. 🔴 `meshlets_per_mesh` is a SCENE-WIDE maximum

The scene cull dispatches `instance_count × meshlets_per_mesh`, and that
second term is the max over every mesh registered — not per instance. In
`dense.scene`:

| | instances | real meshlets | threads paid |
|---|---|---|---|
| cube | 2000 | 1 | 4953 |
| dragon | 24 | 4953 | 4953 |

**98.8 % of a 10 M-thread dispatch is padding**, and it is contagious:
adding one dragon to a scene of cubes multiplies the whole cull by 4953.
Adding a cheap object should not cost more per object.

The fix is the two-level cull every Nanite-shaped renderer has — reject
instances first, expand only the survivors into meshlets — which is the
same pre-pass a render distance hangs off. **No issue yet.**

### 3. ⚠️ The moved-caster list overflows, and the panel says everything is fine

`MOVED_CAPACITY = 256`. `dense.scene` reports `moved=2064`, past which
the scene generation bumps and **every page redraws every frame** —
while the panel still reads a 100 % pool hit rate. The two readings
together look like a working cache, which is the kind of failure nobody
recognises by sight.

The 256 is arbitrary and the panel's reading is misleading. Both are the
issue. **No issue yet.**

### What was NOT wrong, checked against the code

Reported as *"las sombras se ven muy mal"* over a screenshot. None of it
was the shadow path:

- **`render_scale: 50` with `sharpening: 0`** in the project's own
  `.rendersettings`. SGSR 2 reconstructing 880×442 into 1617×884 with
  the RCAS pass at zero — which `settings.rs:508` already calls *"how an
  upscaler earns the verdict we tried it, it looked worse, we turned it
  off"*. Every stair-step on every silhouette is that, and a shadow is
  the finest detail in the frame, so it dies first.
- **Sun azimuth 0°.** The light pointed down the camera's own axis, so
  every shadow fell behind its own caster and 2000 boxes on a white
  floor cast nothing visible. The one shadow on screen was the ball's,
  seen end-on and squashed by perspective into a line to the vanishing
  point.
- `shadow_distance: 200` is **cascades only** and `virtual_shadows` is
  on, so it is never read. `contact_shadow_length: 0.3` m is invisible
  at 30 m spacing, but it is not the cause either.

🔴 **Judge a shadow at `render_scale: 100`, or you are judging the
reconstruction.**

---


## 🎯 The order, decided 2026-08-26 — wire what is already built, then the atmosphere

Two sessions of shadow-page work ended with the page pool stable and
verified on the handheld (#971, #973, #873 all closed). What that week
taught is the criterion for what comes next, and it is not "the biggest
number in the table".

**Both #971 and #973 were features that existed and were not reached.**
The scene epoch was computed and read from a process that never moved it;
the page table's slots were released by passes that no longer walked
them. Neither needed a new technique. Both were a wire that was not
connected, and both cost a day precisely because nothing said so.

So the queue starts with the two remaining cases of the same shape:

### 1. #950 — Hi-Z occlusion culling, built and never called

`dispatch_scene_pool_atomic_hi_z` tests meshlets against the previous
frame's depth pyramid, and **its only caller is the fallback path for
adapters without int64 atomics**. Every path this hardware runs calls the
plain `dispatch_scene_pool_atomic`, which stops at frustum + normal cone.

Occlusion culling exists in this engine and has never executed on the
target. Lowest implementation cost of anything on this list, already
reviewed, and it helps every consumer — main view and shadows alike.

### 2. #949 — the sun has no depth rejection at all

#940 landed Olsson §4's receiver bound and it reaches **lamps only**.
`mark_sun` records nothing and the check sits inside `if !id.is_sun`. The
sun is the consumer that actually rasterises — 17.4 ms of the 41.8 ms GPU
frame in #948 — and it is the one with no rejection.

The mechanism is written, the table word is allocated, and the issue
names both files and both lines.

### Then the atmosphere, in this order and not another

**#976 sky → #977 froxel grid + fog → #731 clouds**, with #978 (wind)
feeding all three and #979 (surfel GI) deliberately last.

Fog and clouds are lit BY the atmosphere. Built before it they get a hack
for a light source and are rebuilt when the real one lands, so the sky
goes first — and its LUT parameterisation has to be settled before
anything consumes it.

🔴 **The grid in #977 is the whole design, and it serves five consumers**:
light clustering (#780), aerial perspective, volumetric fog, clouds
(#731) and the sky itself. Two corrections from the owner got it there:

- Clouds are fog. He shipped Godot's volumetric fog *as the cloud medium*
  in `stellar_delivery` and it was cheap, so there is no second
  long-range grid to design — `volumetric_fog_length` and the Z spread
  are parameters, and they were already used that way.
- The sky is the far slice. This roadmap's first draft claimed froxels
  could not reach it. `projection.rs:92` is
  `perspective_infinite_reverse_rh` and `cluster/grid.rs` is logarithmic
  in Z: there is no far plane and no reach limit. What the LUTs buy is
  not reach — transmittance and multi-scattering are TWO-parameter
  functions every froxel needs, and multi-scattering is an iterative
  solve. They are the froxel pass's inputs, not its alternative.

⚠️ #731's clouds remain switched off at 39.83 ms against a 13.9 ms
budget, and the measured win there is quarter resolution plus temporal
reprojection — which still waits on motion vectors (#481/#732). The
froxel volume replaces the per-pixel march; the reprojection is what
takes it from affordable to cheap.

### What is NOT next, and why

- **#771** — resolved in fact. The sky pass without clouds is 1.22 ms;
  the raymarch was 97 % of it and it is off.
- **#839** — narrowed, not open. The measured project runs
  `contact_shadow_steps: 6` with `dominant: true`: six taps for one
  light, not 224 for fourteen. The uncapped case is the *default*, not
  what ships.
- **#803** — 452 ms of pipeline compilation is real and is a load stall,
  not frame time. Worth doing, not worth doing first.

### ⚠️ Standing hazard, no issue yet

The page table's address space is a function of the light count
(`padded_lights` rounds to 64), so an open world crossing a step clears
the whole table — correct since #973 and expensive. Padding to
`LAMP_CULLS` once would stop the layout moving at all, and it trades
table and bitmap memory nobody has measured.

### 🔴 The test net for all of this is unreliable

`page_marking`'s 33 GPU tests SIGSEGV intermittently under radv,
**verified on clean `development`**, and the lib suite does it too. #725
already describes the shape. Every shadow-page fix this week was
validated by screenshots and unit tests, not by that suite.

---

## Next — the graphics queue, with the budget as the gate

⚠️ **2026-08-17: that 13.89 ms is unverified, and #769 is closed carrying
the caveat.** It was measured on 2026-08-13, before the rule that a
handheld must be warmed for two minutes before it is captured, and before
point cubes (#778), spot maps (#777), the froxel grid (#780) and the
temporal resolve (#481) landed. The captures did not record which scene
they were of, so the issue cannot say whether today's 27.8 ms is a
different scene or a regression.

**What is measured, warmed and settled (10 W / 776 MHz), 2026-08-17,
`many_lights.scene` at 1280×720 with the ball rolling, 11158 frames:**

```
frame 27.83 ms (median)   ·   GPU 24.8   ·   budget 13.9
raster + shade                 21.715  [self 3.655]
├─ shade: compute (half rate)  10.969
├─ taa                          2.883
├─ motion vectors               2.085
├─ shade: upsample              1.533
└─ tonemap                      0.591
shadows 1.153 · sky 0.648 · blit 0.430 · cluster grid 0.156 · cull 0.040
vkAcquireNextImageKHR 23.908 of a 26.670 ms Render
```

Three things in that table decide the order below, and none of them is
what the previous order assumed:

- **The meshlet side is free.** `cull` is 0.040 ms and the geometry pool
  uploads once. VRAM is **817 MiB of 4096** on the device. Nothing about
  meshlets or memory is a suspect.
- **The temporal machinery costs 4.97 ms — 20 % of the GPU** — and it is
  larger than every shadow, the sky, the blit, the grid and the cull
  **combined** (2.43 ms).
- **Shadows total 1.153 ms.** That bounds every shadow-side idea on this
  board, forever, including #477's.

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
| ~~#824~~ | ~~shade in a compute pass, tile's lights in LDS~~ | **Built and measured: 6.6 %.** Fifteen storage fetches per pixel became fifteen per tile and the shading went 35.98 → 33.60 ms. Its value is what it revealed and unlocked, not the 6.6: the raster is **3.74 ms of 37.34**, so shading is 90 % of that pass — and #825 is buildable now |
| ~~#825~~ | ~~shade at half rate, raster stays full~~ | **Built.** Lighting runs at one sample per 2×2 quad, upsampled with the vbuf as the edge guide, so the silhouette on screen is still the raster's — asserted exactly, not approximately. `KOOCH_SHADING_RATE=half`. The device capture is what closes it |
| ~~#826~~ | ~~sample the tile's lights, 15 → 2-4~~ | 🔴 **REMOVED 2026-08-16, with the reason.** A cluster grid is continuous because a light joins a cell exactly when its contribution reaches zero, so a light in one froxel's list and not its neighbour's causes no step. **Any rule that cuts by COUNT breaks that**: the light dropped at the boundary is one that was contributing. Three estimators were built and the device refuted all three — a die over the whole list (squares of pink and cyan, photographed), the frame index in the seed (repaints a ~75x80 px block per frame; no resolve integrates that), and top-K deterministic with the tail carried by scaling (better, still a visible seam). A measured histogram of `many_lights` says why there is no room: the two heaviest lights of a froxel carry **61 %** of its energy and six are needed for 95 %, so there is no weak tail to drop. `light_samples`, `KOOCH_LIGHT_SAMPLES` and the whole `tile_choose` stage are gone — a setting that produces artefacts is worse than an absent one. **What survives is #821**: make weak lights CHEAPER instead of dropping them, which is continuous by construction |
| ~~#851 / #853~~ | ~~a point light's cube was culled with another lamp's frustum~~ | **Merged.** `queue.write_buffer` is not ordered against the encoder — every write queued while a frame is recorded lands before the first command runs. The six cube culls belong to the **face** and are shared by every lamp, so with two casting point lights both cubes were culled against whichever frustum was written last while each was still rasterised with its own matrix. It accounts for all three reports at once: the shadow that dies unless the lamp moves (a moving lamp is redrawn alone, one dispatch), the different picture in each panel (the slot order is the importance ranking, computed from the camera), and the breakage when lamps overlap. Also here: point lights had been running on the **sun's** shadow bias, and `select_point_casters` culled per *view* against state owned by the *stage* |
| ~~#835~~ | ~~a light out of range paid the full BRDF, its shadow cube and the contact march~~ | **Merged (#836).** The froxel is conservative by design and hands the loop lights that reach no part of a given pixel — ~26 of the ~40 in the busiest cell. Nothing asked again at the pixel, so all of it ran and was then multiplied by an irradiance of exactly zero. The cut reuses the `reach` already computed for `specular_floor` and is bit-exact. Half-rate shading on the device went 48.791 → 21.6 ms across the change, with a camera difference in the way |
| ~~#837~~ | ~~submit the scene before acquiring the swapchain image~~ | **Merged, and it bought nothing.** Structurally right — the meshlet stage draws into its own textures and had no reason to be gated on the surface — but the frame was already 0.66 ms from the GPU, not 3. See the refutation below |
| ~~#796 / #819~~ | ~~ReSTIR / Solari~~ | **Ruled out for this hardware.** Solari's world cache alone is 2.65 ms per refresh in Bistro on the author's machine — 19 % of our whole budget, on far faster silicon — and denoising runs through DLSS Ray Reconstruction, which the 890M has no path to |
| **#731** | volumetric clouds, froxel-based | The clouds are **off**, and that is the only reason the budget is met. They cost 39 ms as written |
| **#803** | 452 ms compiling pipelines on frame one | Load time, not frame time — but it is half a second of black screen every launch |
| **#254** | post + auto exposure | The blown-out white floor in three sessions of screenshots. Cheap |
| **#771 / #248** | atmosphere, ported from Bevy | Now worth doing for the sky it gives, not for what it saves: 1.2 ms without clouds |
| **#481** | motion vectors + TAA → **the engine's own upscaler** | **Temporal anti-aliasing built.** Sub-pixel jitter into the raster's projection, motion vectors reconstructed from the visibility buffer with the *unjittered* pair, Bevy's resolve between the radiance and the tonemap. On the strongest 1 % of edges the resolved image carries **0.38** of the squared gradient the unresolved one does. **Off by default** — asset and engine alike, see below. 🎯 The issue stays open because it **grew**: it now owns the engine's own temporal upscaler, Phase 1 below |
| **#536** | ~~vendor plugin backend~~ → optional vendor backends | **Inverted 2026-08-17.** It read *"detect the adapter, load that vendor's SDK, fall back to TAA"*, which makes a vendor's upscaler the path that ships and ours the path nobody tests. Now: ours is the default on every adapter, vendors are optional behind the same trait. `dlss_wgpu` is a genuine adopt — and 🔴 **Vulkan-only**, which on Windows means pinning a backend, plus an SDK it cannot redistribute |
| ❌ **FSR 4, reimplemented in WGSL** | ruled out, 2026-08-17 | Not a licence problem — a **`wgpu` and silicon** problem. FSR 4 is a neural network; `wgpu` 29 exposes no cooperative/subgroup matrix (checked in `wgpu-types-29.0.4/src/features.rs`: `SUBGROUP` only, though naga *does* implement `dot4I8Packed`), and gfx1150 is RDNA 3.5 with no FP8. AMD's own INT8 variant on a **300 W** RDNA 3 part nets **9 %** over native where FSR 3.1 nets **29 %** |
| 🟢 **FSR over FFI** | the route that replaced it | **Calling AMD's shipped library is a different question from reimplementing its algorithm, and it has a better answer.** FSR 3.1's FidelityFX API is a stable C ABI that FSR 4 also uses: one `bindgen` surface covers both. FSR 3.1 builds on Linux with a native Vulkan backend (⚠️ `wine` for the shader compiler) and runs on all three vendors; FSR 4 is Windows-only because AMD ships it as *signed prebuilt DLLs*. See #536 |
| ~~#732~~ | ~~temporal upscaling on both vendors~~ | **Closed as superseded.** One issue describing three things; its phases are done (#481) or split (#481 + #536), and its findings were carried across rather than dropped |

### 🎯 The order, decided 2026-08-19 — the audit first, VSM second, Arm ASR last

With #884 merged, three items were on the table and none of them was urgent.
The user picked the order, and it is A → B → C.

| | | Why here |
|---|---|---|
| **A** | **#885 — a full audit of the tree** | It costs **zero handheld cycles**: no rebuild, no thermal settling, nothing the user has to wait through, so the audit runs while they work on something else. And the ~1400 lines of FSR 3.1 only just landed — auditing them now costs half what re-reading them cold in three weeks does. Scope: duplication, shaders, iteration techniques, entities/components/queries, Rust practice, leaks, GPU resource lifetime, DOD |
| **B** | **Virtual shadow maps — which is #866 *then* #477** | ⚠️ Corrected: **VSM is #477**, and it sits on top of #866, the shared page pool, per Phase 2 / Phase 3 below. What the **game** asks for: shadows that survive planetary scale. It is the largest of the three, and it gets built on the tree A leaves clean rather than on the one that just grew by an upscaler |
| **C** | **#886 — Arm ASR** | A third upscaler, proposed the week the second one landed as *desktop-only*. Arm's **+53 % fps / −20 % power are press material**, not a number this project took, and the source still sits in an unopened submodule. It moves the needle least: the frame budget already closes at 13.89 ms without it |

**What #884 settled, and why C is last.** On the settled OneXFly (10 W, the
Steam → gamescope route, `gpu_busy` 90–99 %), `fsr3` costs **11.682 ms**
against SGSR 2's **1.868** for the same frame — six times, against a whole-frame
budget of **13.9**. FSR 3.1 ships as a **desktop** technique and SGSR 2 stays the
handheld default. ⚠️ The optimisation pass that took `fsr3` from 14.704 to 11.682
is **contaminated and was reported as such**: untouched `shade: compute` fell 22 %
in the same batch and the `fsr3`/`shade` ratio went 2.734 → 2.793 — no attributable
gain. The comparison that decides is the cross-technique one, not the before/after.

### 🔴 One budget was doing the work of two — #889, 2026-08-19

A survey of fifty-three years of SIGGRAPH was run through a single filter:
*does it fit in wgpu, and does it fit in 13.9 ms on a OneXFly?* Sixty-seven
techniques judged, thirty-six discarded. The user's objection is the right
one: **games built with this editor run on the handheld and on a high-end PC,
AMD and Nvidia alike, and the editor itself runs on the desktop.** One budget
cannot judge both.

Sorting the discards by *reason* is what makes it actionable, and it recovers
less than it sounds like:

| Reason | What a second target changes |
|---|---|
| **The API — wgpu** | Less than was claimed here at first, and the correction is below. **Cooperative vectors** genuinely do not exist in wgpu, so neural shading, PSSR and the Neural Light Grid stay out at every tier. **Ray tracing does exist** — see the next section |
| **The budget** | **This is the real recovery.** Stochastic SSR, GTAO at full resolution, volumetric fog on a finer froxel grid, LTC with more lights, full-rate shading, a larger VSM page budget — those were never *wrong*, only wrong **at one tier** |
| A decision already taken | Lumen, MegaLights, VoxelGI/SDFGI/DDGI, lightmaps, TAAU, XeSS — closed on their own merits, unchanged |

**The engine had no way to express a tier**, which is why the survey had to pick
a target and discard against it. **#889** is that gap: a technique declares its
cost per tier, and a preset names the tier. Explicitly — **not** by detecting the
adapter, which is the same mistake #536 was re-scoped to remove. FSR 3.1 is
already a tier and is currently recorded as **a string in a dropdown**.

🔴 **Correction, same day: wgpu HAS ray tracing, and this document said it did not.**
`Features::EXPERIMENTAL_RAY_QUERY` is in **wgpu 29 — the version this engine is
already on** — and it carries acceleration structures (BLAS/TLAS) plus inline ray
queries in WGSL behind `enable wgpu_ray_query`. It is how **Bevy Solari** is built.
So "no GPU gives wgpu ray tracing" was wrong, and every verdict that leaned on it
has to be re-derived from the reasons that actually hold:

- 🧪 **Experimental, and the word is upstream's.** *"May have major bugs"*,
  *"expected to be subject to breaking changes"*. Ray **pipelines** (raygen/hit
  shaders) are still in development; only inline ray queries work.
- 🔴 **The denoiser is the real wall, and it is vendor-shaped.** Solari is *"still
  NVIDIA only in practice due to relying on DLSS-RR"*. FSR Ray Regeneration exists
  and is **DX12-only, no Vulkan**. So on the 9070 XT the rays are reachable and the
  thing that makes one sample per pixel look like an image is not.
- 🔴 **And the cost is against the wrong budget.** Solari on an **RTX 3080** at
  1600×900 upscaled: Bistro **14.06 ms**, PICA PICA 7.96, Dragons 8.58 — of which
  the DLSS-RR denoiser alone is **~6.1 ms**. Bistro on a 3080 is *more than this
  project's entire frame budget*, and the denoiser alone is 44 % of it.

What that re-opens is **not** ReSTIR-class GI on a handheld. It is the family that
needs few rays and little denoising, and it lands exactly where this board already
had a hard problem: **shadows that survive planetary scale**. A ray query has no
cascade to fit, no atlas to page and no four-cubes-for-a-hundred-lights problem —
which is the whole reason #477 and #782 exist. The section *"Why VSM waits for ray
tracing"* below was written before this was checked; it is now a live comparison
rather than a wait.

**Two vendor questions, answered rather than assumed (verified 2026-08-19):**

- 🟢 **DLSS is reachable, and it is #536.** `dlss_wgpu` 4.0.0 wraps it for wgpu on
  **both Linux and Windows — through the Vulkan backend only**, so Nvidia would
  mean pinning the backend on Windows, where wgpu picks D3D12. On Linux the app
  ships `libnvidia-ngx-dlss.so` beside the binary. 🔴 **The NGX SDK cannot be
  redistributed**, so DLSS can never be in a default build — whoever uses the
  editor supplies it. It also emits Vulkan validation errors by an NVIDIA bug,
  which our validation-clean CI would have to except.
- 🔴 **vkd3d-proton does not deliver FSR 4, and it is not close.** FSR 4 is still
  **DX12 only and cannot be integrated into a Vulkan application**; the brief
  open-source release was **withdrawn** and FidelityFX SDK 2.0 ships it as signed
  prebuilt DLLs with no source. The way it works on Linux today is
  `PROTON_FSR4_UPGRADE=1` downloading **`amdxcffx64.dll` into a Wine prefix** and
  injecting it into a D3D12 title. A native ELF binary has no prefix, no PE
  loader and no `d3d12.dll` to intercept. Reaching it would mean a D3D12 render
  path (wgpu only has one on Windows), vkd3d-proton's native build — which its
  own documentation calls *"mostly relevant for development purposes"* — **and**
  Wine on top for the signed DLL. That is shipping a Wine prefix inside the
  engine. On RDNA 3.5 it would land on the glitchy FP16 path, to compete with
  SGSR 2's 1.868 ms.

### 🎯 The order, decided 2026-08-17 — graphics first, and the game waits

The user's call, in their words: *"el juego va a esperar un poco más, ordena el roadmap del
engine para atacar toda esta parte gráfica primero"*. The previous A → B → C is spent: **A
(#785) closed** — the profiler was exonerated by three rulers agreeing within a frame — and
**B (#826) refuted and removed**.

**Phase 0 — four measurements, no code.** Every one is a setting already in
`project.rendersettings` or a capture. They exist first because three graphics ideas in a row
died on their own measurement this week, and each of these bounds an item below it.

| | | The question, and the number it is against |
|---|---|---|
| ~~**1 · #481**~~ | ~~`temporal_aa: false`, warmed, same route~~ | ✅ **Measured, and the answer was not the one asked for.** The shading did **not** grow to absorb it — 10.969 → 10.803 is noise — so the resolve is additive work: frame 27.83 → 22.10, GPU 24.8 → 20.5. 🎯 **The finding was elsewhere: `motion vectors` cost 1.994 ms with its only consumer off.** Gated in #868 — the cheapest whole millisecond on this board, and an `if` rather than a quality trade. ⚠️ The −5.73 ms frame delta is directionally right and imprecise (4523 frames against 11158, a 2069 ms stall, `sky` halved = a different route). Still owed: the visual judgement on the device, including whether the contact-shadow seam returns |
| **2 · #825** | `shading_rate: 1` against `2` | Half-rate shading costs a **1.533 ms** upsample to save part of a 10.969 ms dispatch. Nobody has measured the pair as one line |
| **3 · #839** | `contact_shadow_steps: 0` | The control that has never been run. ⚠️ **Premise corrected**: the shipped settings are 6 steps with `dominant: true`, not the 16 × ~14 the issue was written against, so expect **under a millisecond** |
| **4 · #865** | a scene with the same coverage and far fewer vertices per pixel | The shading pass fetches **96 bytes of vertex per pixel** (3 × 32 B, unpacked). Whether that is 3 ms or 0.3 depends on cache hit rate, and packing before knowing is how the last three died |

**Phase 1 — the engine's own temporal upscaler: #481. Third-party ones later (#536).** 🎯
🎉 **SHIPPED 2026-08-18 (#874, v0.4.0): steps 1–4 built and the handheld budget closes — 40.7 ms
to 13.9, GPU 24.8 to ~8.4, 72 fps at 76 % occupancy.** Full numbers below.
**Promoted here 2026-08-17, ahead of #866**, and settled after two corrections in one afternoon —
both recorded because the reasoning matters more than the conclusion.

The user's call: *"si no podemos usar los escaladores, lo ideal sería hacer el nuestro con la data
que encontremos, y más adelante buscamos usar los de terceros"*.

⚠️ **The premise is not quite right and the decision survives it.** FSR **3.1** *is* usable — MIT
source, native Vulkan backend, runs on all three vendors. Only **FSR 4** is out of reach, being
DirectX 12 only. So this is a choice rather than a forced move, and it is the right one for
reasons about this project rather than about FSR: the SDK's shader compiler needs **`wine`** on an
atomic distro, `as_hal::<Vulkan>()` is `unsafe` and pinned to **wgpu 29**, an external signed
library has to be located and redistributed, and ours reaches WebGPU and Metal where a native
library backend reaches neither. The one that decides it: **a bug in ours is a bug we can fix.**

🔴 **What is given up, stated rather than discovered: quality.** *"Improved temporal stability,
less flickering, ghosting reduction"* is the changelog of a single FSR point release. Ours will not
match years of tuning, and the honest bar is **better than the TAA we ship today, at a lower
render resolution** — not *as good as FSR*.

⚠️ **And an earlier version of this section had it backwards.** It read *"no crate covers this,
therefore write it"* — the bottom row of this project's dependency policy, reached by asking *is
there a wgpu upscaler crate* (no) instead of *can AMD's C API be bound* (yes, and FSR 3.1's
FidelityFX API is a stable C ABI). The conclusion is the same; it is now reached on purpose.

**Six steps, and the first three ship one at a time and improve today's TAA even if the upscaling
never lands:** 16-phase jitter, motion vectors dilated by closest depth, history rejection by
variance in YCoCg. Then step 4, the resolution split, which is where the real risk is because it
changes what "a pixel" means to every pass downstream. Then RCAS and feature locking — 🔴 **not
optional polish**: without them the result reads soft, which is exactly how this lands as *"we
tried it, it looked worse, we turned it off"*.

### 🎯 And "based on FSR" means **transliterated, not inspired** — decided 2026-08-17 (option C)

Two things were verified in the vendored sources rather than assumed, and together they change how
ambitious this can be:

| | Verified where | What FSR needs it for |
|---|---|---|
| **`SHADER_F16`** | `wgpu-types-29.0.4/src/features.rs:1657` | Its `FFX_HALF` path — packed half math, which most of the implementation leans on |
| **`SUBGROUP` + `subgroupAdd` &c.** | `naga-29.0.4/src/front/wgsl/parse/conv.rs:379` | Wave-level reductions |
| `dot4I8Packed` | `naga-29.0.4/src/back/spv/block.rs:1452` | DP4a — not needed by the upscaler, only by a network |

**Both things that would have blocked a direct port are present.** So FSR 3.1's passes can be
transliterated to WGSL with **its own tuned constants**, which are the part nobody can guess, and
the licence permits it outright: MIT, with attribution — AMD's copyright header retained in the
ported files and the MIT text shipped in a `NOTICE`.

**The user's order is C**, and the ordering matters more than the ambition:

1. ✅ **Step 1 first** (16-phase jitter). Both routes need it. **Built** — see below.
2. ✅ **Steps 2–3 next** — they produce a visible improvement in the TAA that ships *today*, judged
   by eye on the handheld in the same session. **Built** — see below.
3. ✅ **Then the transliteration**, arriving with the input infrastructure already validated. 🔴 That
   is where ports die — in the inputs, not in the shaders. **Built** — and it did not die there,
   which is the plan's one real prediction that held.

#### 🎉 The result, measured on the OneXFly (2026-08-18) — **the budget closes**

SGSR 2 at Performance (50 %), `many_lights.scene`, TDP pinned at 10 W:

| GPU scope | before (v0.2.44) | **after** | |
|---|---|---|---|
| `shade: compute (half rate)` | 10.969 ms | **2.461** | **4.5x** |
| `motion vectors` | 2.085 | **0.469** | 4.4x |
| `shade: upsample` | 1.533 | **0.410** | 3.7x |
| the resolve | `taa` 2.883 | **`sgsr2` 1.868** | cheaper **and** it reconstructs |
| `shadows` · `cluster grid` · `blit` | 1.153 · 0.156 · 0.430 | 1.203 · 0.134 · 0.398 | ← unchanged |
| **GPU total** | **24.8 ms** | **~8.4 ms** | **~3x** |

**72 fps stable, `gpu_busy` 76 %.** Before this the device sat at 93–99 % occupancy, power-limited
to 1141 MHz of 2900, and still missed the budget. It now makes it with a quarter of the GPU spare.

🎯 Everything that costs per pixel fell between 3.7x and 4.5x; everything that does not depend on
resolution stayed identical. That is the independent check that the size sweep was complete.

⚠️ `vkAcquireNextImageKHR` is 11.2 of the 13.9 ms — the frame is **waiting on vsync**, and the CPU
side does its work in 1.6 ms. And **p99 is 28.29 ms**, which `read_capture` reports is *not*
explained by GPU work (r = 0.18): with the GPU at 76 %, whatever moves the slow frames is elsewhere.

#### ❌ TAAU — built, measured, and removed. Do not re-litigate

Ours, the same shader as the resolve: at 1:1 every gather weight collapses to one and it IS the
plain resolve (measured at 0.0003). It lost on **both** counts — **4.482 ms against SGSR 2's
1.868**, frame 16.29 against 13.91, `gpu_busy` 98 % against 76 %, and jagged edges.

🎯 **The cost was never the taps.** Cutting nine to five bought 13 %. Counted per output pixel,
**TAAU issued 19 fetches where SGSR 2 issues 7**: SGSR 2 builds its variance box from the SAME five
taps it already read for the gather and reads history with one bilinear tap, where ours sampled
three independent neighbourhoods — five for a Catmull-Rom history, nine for the box, five for the
gather. Closing that means restructuring the resolve, and it would still lose on image.

**Removed entirely** rather than left as a worse entry in a menu. TAAU is the CATEGORY and SGSR 2 is
an implementation of it, and a year of somebody tuning constants beat starting from a better
rejection test. **That is what turns "transliterate rather than invent" from an opinion into a
measurement.**

#### 🎯 The size sweep, and the rule it exposed

The resolution split broke four things that had never had to state an assumption before, because
the render size and the presented size had always been the same number:

| | |
|---|---|
| **The froxel grid** | 🔴 Found **by the owner, from the picture** — the lighting broke into blocks of wrong colour. The shading pass indexes it from a fragment coordinate produced at RENDER resolution |
| The LOD target | Compares a meshlet's projected error against a PIXEL, and the pixels that exist are the rasterised ones |
| Both Hi-Z pyramids | They are mip chains of the depth buffer, which shrinks |
| The shading dispatch | The pass the whole scale exists to make cheaper |

🎯 **Everything that DERIVED its size from a texture was already correct; everything that RECEIVED a
size as a parameter was wrong.** Contact shadows read `textureDimensions(depth)` and needed nothing.
The projected shadows rasterise from the light at their own resolution and never look at the screen.

#### ⏭️ What is left of #481

- **Step 6: feature locking.** ⚠️ Written for a resolve of our own shape, and the resolve that won
  is SGSR 2 — which has no locks and reconstructs thin geometry through the Lanczos weight
  instead. Grafting FSR's lock buffer into a transliterated shader is the opposite of the rule
  that just paid for itself. **Judged on the device first**: if a one-pixel wire dissolves at
  Performance, the mechanism is decided then, and against what SGSR 2 already does.
- **FSR 3.1** as the second backend, which is what the seam was built for.
- ⚠️ **`MipBias` is still missing**, and the reason it was never going to work has been fixed
  first: 🔴 **the engine had no mip chains at all.** Every texture was uploaded with
  `mip_level_count: 1` since the first PR that had textures, so a negative LOD bias would have
  selected level zero — which is what it already selected. The setting could not have moved a
  pixel, and it would have been debugged as an upscaler problem. Chains land in this PR; the bias
  is next and now has something to bias.

#### 🔴 What judging RCAS actually needs — found by trying to judge it

The owner set `sharpening` to 0, 60 and 100 in the editor and saw no difference. Two findings,
both of them about the input rather than the pass:

1. **The scene was a white room.** Flat white Suzannes on a flat white floor at
   `ambient_intensity 300`, half the frame clipped at 1.0 and no texture anywhere. RCAS amplifies
   *local* contrast: where the five taps agree, the filter is the identity by definition. Measured
   on the test scene the pass moves the mean gradient by 33 % and the worst 0.1 % of pixels by
   21/255 — a real effect, and invisible on a surface that has no detail. Hence the prototype
   textures, which are now shipped.
2. 🔴 **There were no mip chains.** See above. A grid texture on that floor would have boiled, and
   the boiling would have been blamed on the upscaler.

🎯 **The lesson is the one #481 keeps teaching: a renderer feature judged against nothing measures
nothing.** The order is chains → bias → judge, and it was arrived at by looking at a screen and
finding the answer was not in the pass.

#### ✅ Step 5, as built — RCAS, and where a sharpening pass has to sit

`sharpening` in `.rendersettings`, 0..=100, `KOOCH_SHARPENING` on top of it for a capture run.
One full-screen pass, its own `rcas` scope, **ported from Bevy's `robust_contrast_adaptive_
sharpening.wesl`** — which is FidelityFX FSR 1's `ffx_fsr1.h`, MIT, and carries a constant
(`0.1875`) that someone spent a year finding.

🔴 **It runs AFTER the tonemap, and that is the whole design decision.** RCAS is adaptive because
it solves for the filter weight at which the signal would clip out of `{0, 1}`; handed linear
radiance in the hundreds, that limiter stops limiting. The same lesson the resolve learned about
the exposure, arrived at from the other side — there the fix was to bring the arithmetic to the
data, here it is to put the pass where the data already is. The cost is one `Rgba8Unorm` target at
output resolution, which the tonemap writes into instead of into the window while the pass runs.

🎯 **Two of the four GPU tests could not fail as first written, and both were caught by breaking
the shader on purpose rather than by reading them:**

| First version | Why it could not fail | What replaced it |
|---|---|---|
| The border's mean brightness | Removing the bounds clamp does not darken the edge — a zero neighbour drives `mn4` to zero, which drives the solved lobe to **exactly zero**. The number did not move in the third decimal | The bottom row's own **gradient**: with the clamp it sharpens like every other row, without it, it is the one row that never does |
| The frame's mean brightness | Raising the limiter 53× moves it by **0.1 %**. RCAS redistributes contrast, so the mean is what it is designed not to touch | The **p99.9 per-channel change**: 21 of 255 with the cap, 43 without, worst pixel 29 against 136 |

⚠️ **The default is 0, and the engine's own capture project is the one that must not leave it
there** — a scale below 100 without this is half the change, and it is the half that gets judged.

#### ✅ Step 1, as built — the period is now a function of the ratio

`JITTER_PERIOD` is gone. In its place `jitter::phase_count(render_width, display_width)`, which is
FSR's rule with our base: **the count scales with the square of the ratio**, because what the
sequence has to cover is an area. At 1.5× every output pixel is fed by 1/2.25 of an input one, so
reaching the same sub-pixel density takes 2.25× the samples — 36, not 24. Scaling it linearly is
the plausible-looking mistake, and it reads as *"FSR is soft"* rather than as a jitter bug.

Two choices in there are not FSR's, and both are one line to reverse:

- 🔴 **Our base is 16 where AMD's is 8.** At 1:1 — which is every capture that exists today — 8
  phases against a minimum blend rate of 1/64 means a pixel that never rejects its history
  averages each of eight points seven times over instead of covering more of the pixel. **Set
  `JITTER_BASE_PHASES` back to 8 if the transliterated passes disagree**: their accumulation
  constants are tuned against AMD's count, and that count is one of the things in the port that
  cannot be guessed.
- **A ceiling, `JITTER_MAX_PHASES` = 144** (3×, FSR's Ultra Performance preset). Not tidying: a
  render width of zero — a minimised window, a target queried a frame early — squares to tens of
  millions of phases, and a period that large is a sequence that **never repeats**. The
  accumulation would then drift instead of closing a cycle, which is soft in a way no capture
  explains.

The split itself is not built: `Vbuf64Stage::jitter_phases` passes its own width as both
arguments, so today's answer is 16 and **the second argument is the single thing that changes**
when step 4 separates them. `Jitter::at` takes the **render** target's size — the camera is
jittered by a fraction of the pixel it actually rasterises, and measuring it against the output
would shrink every offset by the ratio.

#### ✅ Steps 2–3, as built — and one premise in the plan was wrong

🔴 **The plan said step 3 was "today's resolve uses a neighbourhood clamp, which is the crude
version of this". It was not.** The resolve has clipped against a YCoCg mean ± σ AABB since it
was written — Playdead's, clip and not clamp, at one sigma for the reason recorded in
`taa.wgsl`. The step was written from memory rather than from the shader. What was actually
missing was the *other* half of step 3, the **disocclusion mask**, and that is what got built.

| | before | now |
|---|---|---|
| **Dilation** | 5 taps at **2** texels (Karis' cross) | **9 taps at 1** (FSR's 3×3) |
| **Variance clip** | YCoCg, 1σ, clip-not-clamp | unchanged — it was already there |
| **Disocclusion** | ❌ nothing | reversed-Z depth compared at the reprojected address |
| **Confidence counter** | read at the pixel's **own** address | read at the reprojected one |

**Why the dilation narrowed rather than widened.** At two texels the closest-depth winner can be
a surface that does not touch this pixel at all, so a thin foreground object hands its velocity
to a two-pixel skirt of background — the object drags a halo. Step 4 makes that worse by
construction: after the resolution split one input texel is 1.5 output pixels, so a two-texel
reach is three.

**Why the disocclusion mask is not redundant with the clip.** The clip catches a history whose
*colour* no longer fits its neighbourhood. It cannot catch one whose colour fits perfectly and
belongs to a surface at a different distance — a wall revealed from behind a pillar, in front of
a wall of the same shade. That one survives the clip and ghosts for the twenty frames the blend
takes to forget it.

🎯 **Reversed-Z pays for itself here.** The infinite-far projection maps `z = near / distance`
exactly, so the **ratio of two depths is the ratio of the two distances**. The whole test is a
divide: no linearisation, no near plane in a uniform, nothing to keep in sync with the camera.
The depth rides in the confidence target's second channel — `R16Float` → `Rg16Float`, two bytes
a pixel, no new attachment.

⚠️ **The tolerance is 10 % and deliberately loose.** It has to separate a foreground silhouette
from its background (tens of per cent) without firing on a camera walking forward (a few per
cent per frame at any speed a player uses). Tighten it and the resolve rejects everything the
moment the camera moves — which still costs both passes and reads as *"TAA does nothing here"*
rather than as a bad constant. `a_slow_pan_keeps_accumulating` exists to fail if that happens.

🔴 **This is the cheap test, not FSR's.** FSR reconstructs the previous frame's depth into the
current grid — an extra pass with atomics — and is exact under any camera motion. Ours compares
against this pixel's own depth, so axial camera motion shows up as a small error everywhere at
once. That is what the loose tolerance absorbs, and it is the thing to revisit if the mask ever
misbehaves on the device.

⚠️ **Not judged by eye yet.** These are the two steps whose payoff is visual, and the handheld
run has not happened.

🔴 **And a test-harness trap that cost a debugging round:** `tests/common` hands every case in a
binary the **same** device, so four cases running at once segfault radv intermittently — and
pass reliably under `--test-threads=1`, which is the worst possible way to be told. That binary
is now serialised behind a mutex, the same pattern `gpu_scopes.rs` already used.

⚠️ **The transliteration's real risk is that it has no oracle**: nothing to diff against, so
validation is by eye. If it ever degenerates into *"it looks wrong and I do not know why"*, the way
out is to build the deferred FFI backend (#536) **as a reference to compare against** — that is a
debugging tool, not a change of plan.

🔴 **And nothing below us is going to solve this.** `wgpu` is a GPU API abstraction and ships no
render techniques by design — no TAA, no shadows, no upscaling, and it never will. The layer that
would is Bevy, and Bevy is **binding vendors rather than building one**: DLSS integrated through
`dlss_wgpu`, FSR / XeSS / MetalFX noted as *"would not be a challenge to integrate"* later, and
frame interpolation and **dynamic resolution scaling explicitly not planned**. There is no port
target coming. What `wgpu` does give us is the three primitives in the table above, which is
exactly enough.

**What survives from the FFI plan, and it is not small:** #536's trait, `UpscalerCaps` and startup
selection are **built in Phase 1 with ours as the first backend behind them**. Building the seam
while there is one implementation is what makes the second one a day's work; building it after is
a refactor. And `UpscaleInputs` stays **FSR 3.1's six** — jittered colour, depth, motion vectors,
jitter offset, `exposure`, reset — which is what makes *"más adelante"* a configuration change.

🎯 **The corpus is written down in #481 with a licence column**, because that column is the trap:
FSR 2/3.1's HLSL is **MIT and copyable** and is the primary reference; Bevy's is MIT/Apache;
Karis 2014, Playdead's INSIDE (where the YCoCg variance clipping comes from), Salvi and the Decima
talk are papers to implement from. 🔴 **UE5 TSR is the closest existing thing to this exact
category and is under Epic's EULA — read the talks, never the source.** FSR's is MIT and solves
the same problem, so there is no reason to go near the encumbered one.

🔴 **FSR 4 is DirectX 12 only, and that decides everything about it.** Not a licence limit and
not a hardware one: AMD has shipped no Vulkan backend for FSR 4, so *"it cannot be integrated
into games that use the Vulkan API"*. Which means:

| | FSR 3.1 | FSR 4 / 4.1.1 |
|---|---|---|
| **Windows** | ✅ VK or DX12, MIT source | 🟢 real path, via `wgpu`'s D3D12 backend and `as_hal::<Dx12>()` |
| **Linux — this machine and the OneXFly** | ✅ native VK backend | ❌ no D3D12 on Linux, so none |

So FSR 4 is **a desktop-Windows image-quality feature that costs a second `unsafe` interop
path**, and it does nothing for the 13.9 ms problem, because the device the budget is about runs
Linux. It is **deferred, not refused** — the third backend behind the trait, after ours and after
FSR 3.1. ⏭️ The single fact that would move it up is AMD shipping a Vulkan FSR 4.

⚠️ Adrenalin's automatic upgrade of an FSR 3.1 integration to 4.1.1 is **DX12-only too**, which
cuts both ways: no free FSR 4 on a Vulkan integration, and no surprise change to an image we
validated. And *"FSR 4 on Linux"* as it exists today — VKD3D-Proton 3.0, with an INT8/FP16
fallback its own authors warn costs *"significant performance overhead and noticeably reduced
image quality"* on RDNA 2/3 — requires being **a Windows binary under Wine**. A native ELF has
none of it.

It is also, on the numbers, **the largest single lever on this board**. Rendering at 67 %
linear is 44 % of the pixels, and per-pixel shading is what dominates the frame:

| | today | at 44 % of the pixels |
|---|---|---|
| `shade: compute (half rate)` | 10.969 ms | ~4.9 |
| `shade: upsample` | 1.533 | ~0.7 |
| temporal resolve | 2.883 (`taa`) | ~2.0–2.5, replacing it |
| **net** | | 🎯 **≈ −5 ms of 24.8** |

Every other item is bounded far below that: all shadows total **1.153 ms**, the cull 0.040, the
grid 0.156. And unlike them it **compounds** — every per-pixel cost added afterwards is bought
at 44 %.

⚠️ **The saving is arithmetic, not a measurement.** It assumes the resolve costs what today's
`taa` costs; a real FSR 3.1 may cost more, and on this iGPU that has to be measured before the
−5 ms is quoted as a result.

⚠️ **It is a reconstruction, so it is still a trade.** 1280×720 *output* is a hostile case
because there is little detail to reconstruct from. The device decides. Which is why #254 now
also owns the **non-temporal** fallback — `None | Spatial | Temporal`, with CMAA2 over SMAA 1x
for a compute frame with no LUT assets to ship.

**Phase 2 — the structure the light and shadow side needs: #866.** One page pool (bricks out of
an atlas, not an octree), clipmap levels for range, and invalidation by reach rather than by
the world. It is not three features, it is one structure with three consumers — and this
project has already built and measured the small version of each part: the froxel grid (#780)
produces "which lights need detail where", and #847 proved invalidation-by-reach is worth
**2.781 → 1.153 ms** at cube granularity. **Demoted below #481** because its payoff is scale
and memory, and the frame's problem today is cost per pixel.

🔴 **Measured 2026-08-20, and it settles Phase 2's first move (PR #927).** The census —
`cargo run --example measure_shadow_pages` — is built to *Unreal's* configuration read off
[the UE 5.8 VSM
documentation](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine)
rather than quoted from this board: 16k virtual, 128-texel pages, clipmap levels 6..22 for
the sun and a mip chain for locals, levels picked *"by projecting the size of the screen
pixels into shadow map space"*.

**The marking input is the decision, and the sun is where it shows:** on
`many_lights.scene` the sun's clipmap residents **15 770** pages when every cell of the
frustum is marked — 277x the theoretical floor — and **118** when only the cells containing
geometry are, about twice that floor. A froxel is a box of mostly empty air, and Epic says
it in one sentence: *"depth buffer analysis is used as the primary method of marking pages
that are needed to render"*. So #866's own *"read it off the froxel grid that already runs"*
is the sentence this refutes. Two sweeps run as predictions moved that count 31 % and 25 %
where the surface filter moves it **133x**, so it was an area rather than a dicing artefact.
**Neither page size nor virtual size matters**: flat within 2 % across 64/128/256-texel
pages, *identical* across 4k/8k/16k virtual maps.

🎯 **What it says about the engine as it stands.** For the content shipping today — five
casting lights — the pool is **14.1 MiB against 152**: eleven times less, same image. ⚠️ But
local lights barely benefit from better marking (**1.2x**), because `range` already bounds
them to the cells near geometry: a hundred casting local lights cost **424.9 MiB**, or
**6 916 pages** — past Epic's open-world recommendation of 6 144 and into the band they say
thrashes. 🔴 And `r.Shadow.Virtual.MaxPhysicalPages` defaults to **4096 pages = 256 MiB**,
which is *more* than this engine's 152 MiB of fixed allocations: **the pool is not
inherently smaller, it is adaptive.** It replaces a cap of four slots with a cap of memory,
which on a handheld is still a cap — so the next lever is the **density target**, not the
marking, and any summary promising that a hundred casting lights become free is wrong.

🔴 **The table cannot be flat, and that decided Phase 2's second move (PR #930).** With
128-texel pages, a mip chain per cube face and a 17-level clipmap, 101 lights and a sun
address **28 409 856 virtual pages**. One bit each is the 3.4 MiB mark bitmap and that is
why marking was affordable; one `u32` each is **108 MiB — 42 % of the 256 MiB pool it would
index**, to describe pages 99.99 % empty. It also kills the obvious allocator: a sweep over
the virtual space is a 28-million-thread dispatch to find ~2000 set bits. So the table is
sized to what is **resident**: open addressing over `2 x pool_pages` entries, **64 KiB** at
Epic's 4096-page pool, which is what UE5 does too. The allocation itself is free —
`mark_bit` already reports the one thread that flipped a page's bit, so claiming a slot
there is an `atomicAdd` on a rare branch. ⚠️ The atlas texture is **not** allocated yet: 256
MiB that nothing writes until the raster lands.

🎯 **The raster landed for the SUN, and the seam is the cull (PR #931).** Four passes: cull
per clipmap level, compact the table into a dense per-level list, expand into
`(page, meshlet)` pairs, and **one** `draw_indirect` into an atlas where every page is a
sub-rect — 1681 pages as one render pass instead of 1681. 🔴 **A cull is per view, and that
is where the sun stops and the locals begin**: the clipmap is 17 views, 101 point lights
with six faces and an eight-level chain are **4848**, and the LOD selector is a two-pass
reduction over the meshlet DAG. Local pages are marked, allocated and **reported as not
drawn**; drawing them needs the cull moved onto the GPU as one multi-view dispatch. Pool
default dropped to **2048 pages = 128 MiB**, against the **152 MiB of fixed allocations
standing today for four lights**. ⚠️ No cross-frame caching yet, and nothing samples the
atlas — that is #477.

🎉 **#477 cerrado: las páginas se samplean y la sombra del sol sale del pool (PR #932).** El
lector camina niveles en vez de recalcular cuál se marcó — reproducir esa aritmética sería una
TERCERA copia, y un nivel de más es un lookup que **falla**, o sea una sombra que desaparece en
lugar de una a escala equivocada. Cualquier página residente que contenga el punto sirve.
⚠️ **El ráster y el shading son UN fragment shader fusionado**, así que no hay depth para marcar
hasta que el sombreado terminó: lo que se samplea es lo del frame anterior. El modo de falla es el
correcto — una página nueva sale **iluminada** un frame, no mal. Los taps son `textureLoad`
clampeados dentro de la página: un filtro de hardware no sabe dónde termina, y los téxeles de al
lado son de otro nivel del clipmap. **Todos los knobs pasaron a `.rendersettings`**, grupo
`Shadows: virtual pages` junto a `sun cascades` y `contact`. 🔴 **El rate de marcado se eliminó**:
decidía cuántos hilos, ahora decide qué páginas EXISTEN.

🔴 **2026-08-24 (12) — LA MEDICIÓN EN LA OneXFly: EL TRACK DE VSM CUESTA ~34 ms Y NINGÚN
SCOPE PODÍA VERLO (#948).** `many_lights` por Steam→gamescope, caliente, 20 W: frame **mediana
46.56 ms (21.5 FPS)**, p99 98.30. La captura declaraba **11.4 ms de GPU, plana en todos los
deciles**; `drm-engine-gfx` declaraba **969 ms/s — 93% del wall, ≈45 ms/frame**. `fdinfo` por
proceso cierra la discusión: `roll_a_ball 968.99`, `gamescope 0.00`, `mangoapp 0.00` — **no es
el compositor y no es remote play, es nuestra propia cola que no drena**, y por eso
`vkAcquireNextImageKHR` bloquea 69 ms en los frames lentos. El encoder del frame abría
exactamente DOS scopes de GPU (`cull` y `raster + shade`) y grababa `record_page_marking`
**entre los dos y dentro de ninguno**: el mark, los 17+ culls de clipmap, la compactación, la
expansión y el draw caían bajo ningún nombre. Peor, la captura DESVIABA: su chequeo de
correlación decía "la variación no la explica el trabajo de GPU" (r = 0.198) — cierto del
trabajo **medido**, y se lee como "buscá en otro lado" cuando tres cuartos del trabajo no
estaban en la serie. `4bea7050` es SOLO instrumentación: `shadow pages` → `page mark` +
`page raster` → `page cull` / `page expand` / `page depth` (los cuatro escalan con cosas
distintas — niveles, páginas residentes, pares, téxeles cubiertos — así que un número sobre el
conjunto no dice cuál creció). ⚠️ **Dos tests pasaban por encima del agujero**:
`the_page_passes_are_profiled` decía cubrir "los dos entry points que graban el trabajo de
GPU" y sólo afirmaba `profiling::scope!`/`#[profiling::function]`, que miden la GRABACIÓN, no
los dispatches; y `gpu_scopes.rs` deletreaba a mano el bundle del path R64 en vez de llamar a
`all_required_features()`, se comía `SHADER_F16` y los **tres** tests del path que toma la
OneXFly morían en `create_shader_module` sobre `fsr3_accumulate` — el camino que se estaba
midiendo no tenía un solo test verde. ⏭️ **NEXT = redeploy + recaptura**, y recién ahí se sabe
cuál de los cuatro pases se come el frame. `KOOCH_SHADOW_POOL_PAGES` (default 2048, rango
4..8192) es la palanca de A/B. 🔴 **`virtual_shadows` NO se flipea a ON hasta tener ese
número.** Aparte, 2874 MiB de VRAM en una handheld quieren su propia mirada.

🎯 **2026-08-23 (11) — EL PRÓXIMO FRENTE, DECIDIDO: EL JUEGO TIRA DEL ENGINE.** Cerrado el
hilo VSM (falta solo la medición en la OneXFly, que corre el user con su build), lo que sigue
es **A: mecánicas + un nivel real**, con dos issues nuevas que lo habilitan: **#946 — CSG de
blockout** (clase Godot CSG, NO ProBuilder: primitivas + booleanas → malla que cae directo al
pipeline de meshlets existente; investigar crates de booleanas ANTES de escribir una — es un
cementerio de edge cases) y **#947 — colisión y oclusión de cámara** (clase Phantom
Camera/Cinemachine: spring-arm con shape-cast de Rapier — no un ray, la cámara tiene near
plane —, whiskers de oclusión, damping correctivo separado del de follow; el sistema de
cámaras quedó incompleto y un nivel real lo va a hacer notar de inmediato). **GI confirmada
en el roadmap**: es **#450 (surfels)** — el user la nombró GIBS (EA SEED 2021), que SÍ usa
raytracing para la radiancia por surfel pero con presupuesto de rayos desacoplado; 🔴 **el
engine NO es solo low-end**: en high-end (9070 XT) el update de surfels puede usar ray
queries de wgpu (`EXPERIMENTAL_RAY_QUERY`, Vulkan), en la handheld la misma estructura se
alimenta de un sampler barato (SDF #449 como far-field) — una estructura, dos proveedores de
rayos, el preset (#889) elige. ShaderForge-like: descartado por ahora (la tool más cara, un
solo artista que ya escribe WGSL). Pendientes de perf que siguen vivos detrás de esto: #824
(compute shading con luces en LDS, el #1 medido de #823) y el flip del default de
`virtual_shadows` tras la medición.

🎉 **2026-08-23 (10) — LAS SOMBRAS CLÁSICAS DEJAN DE RETENER SU MEMORIA BAJO LA VSM (#945).**
Lo que quedaba del defecto #5 era la MEMORIA (los draws ya estaban gateados: `draw_cascades`
mató los 0.33 ms, y spots/points van con listas vacías bajo pages): con sol presente,
`nothing_casts` era falso y el atlas de 64 MiB + 6 MiB por cubo quedaban alocados para un
lector que ramifica a páginas. No pueden ir a CERO — el bind group del shading necesita views
vivas y wgpu rechaza texturas de 0 layers — así que van a un TOKEN: atlas en el piso del
clamp (256), UN cubo de 16 téxeles → **<0.5 MiB donde había 88**. La decisión es una función
PURA (`classic_shadow_alloc`) y la clave de release pasó de un `u32` de téxeles a una tupla
`ClassicAlloc` — el test `a_floor_sized_atlas_still_swaps` clava el edge que el u32 pelado
perdía: un autor con cascadas YA en 256 no habría liberado los cubos al togglear. La puerta
de resize-release existente hace el swap en ambos sentidos sin código nuevo. Con esto el
frame bajo VSM paga SOLO la VSM → la medición en la OneXFly mide lo que dice medir.

🎉 **2026-08-23 (9) — EL BOUND DE RECEPTORES (#940): EL PLAN OLSSON QUEDA COMPLETO.** PMCD
de Olsson §4 a granularidad de PÁGINA (más fino que el per-face del paper): cada sample que
marca la página de una lámpara es un receptor y el marking hace `atomicMax` de su distancia
RADIAL a la luz en la 5ª palabra de la tabla (`PAGE_CELL` 4→5; floats positivos bitcast a
u32 ordenados = atomicMax sobre distancias); `age_view` la borra cada frame (0 = sin datos =
nunca rechazar → los rigs plantados y el sol quedan intactos). La compactación la copia a
`page_list` (vec2→vec4 — el expand está EN el límite de 8 storage buffers y no puede bindear
la tabla), y la expansión de lámparas gana UN rechazo: caster cuyo punto MÁS CERCANO queda
detrás del receptor más lejano de la página no ocluye nada que el frame sombree (radial =
conservador; la rotación del spot preserva longitud → una comparación para ambos kinds).
Contador `depth_rejected` en la línea de pair-tests del panel — el número que dice si la 5ª
palabra paga su memoria. Test `a_caster_behind_every_receiver_pairs_nothing`: dos corridas
idénticas (bound plantado vs 0) y el contador tiene que IGUALAR los pares que desaparecieron
(cerró exacto a la primera). Con esto, TODO el plan derivado del paper está implementado:
cull jerárquico (#939) → caché de contenido (#477/#866) → filtro (#941) → prioridad (#942)
→ bias (#943) → gate (#944) → max-depth (#940). Lo que sigue es MEDIR.

🎉 **2026-08-23 (8) — LAS LUCES QUE NADIE PUEDE RESOLVER NO CASTEAN (#944).** El gate de
cobertura de Epic (`PruneLightGridCS`), como UNA comparación dentro del loop de marcado que ya
tiene todos los operandos: una luz local cuyo rango ENTERO proyecta bajo `shadow_min_pixels`
de radio en pantalla (nuevo knob en `Shadows: virtual pages`, default 8 px, 0 = off) no marca
páginas — sigue ILUMINANDO, los lectores caminan su chain, no encuentran nada y devuelven
lit, y recupera su sombra el frame en que la cámara se acerca. El sol nunca se gatea (no
tiene radio) y el gate yerra hacia castear (mide la esfera de rango, no la parte iluminada).
El paint del debug aplica el mismo gate. Contador nuevo `culled` (pares rechazados) en la
línea del censo del panel. Tests: `a_tiny_light_casts_nothing` (lámpara de ~13 px bajo gate
de 32 → 0 residentes, `culled > 0`, `pairs` idéntico — el gate corta el marcado, no el walk
del grid) + `shadow_min_pixels_reaches_the_settings` (la lección del defecto #1: un setting
que el frame no alcanza se shippea inerte). Con esto la cadena de #942 queda completa:
prioridad (quién) → bias (cuánto) → gate (si siquiera).

🎉 **2026-08-23 (7) — EL BIAS DE RESOLUCIÓN HACE QUE LA DEMANDA QUEPA (#943).** Feedback
GPU-only, cero readback: `bias_view` (1 hilo, al final del marking) lee el cutoff del plan y
mueve un bias persistente por vista un paso por frame — bajo presión las LOCALES pagan primero
(hasta 4 niveles), el sol recién cuando ellas no tienen más que dar (hasta 2); cada nivel es
un cuarto de las páginas. Los lectores NO cambiaron: ambos caminan su chain desde lo fino y
toman la primera página residente, así que un marcado más grueso es simplemente lo que
encuentran (el debug paint aplica el mismo bias o pintaría páginas que el marking nunca
eligió). La vuelta es de dos vías y esa asimetría ES la histéresis: si la aritmética PRUEBA
que un nivel más fino entra (slack ≥ 3× la demanda de esa parte) baja al instante; donde no
puede probarlo — los niveles gruesos del clipmap NO cuadruplican, el ×4 sobre-bloquea, lo
midió el test — PRUEBA un paso tras 16 frames de paciencia y el raise ordinario revierte el
trial fallido al frame siguiente (las páginas gruesas aún residentes atajan a los lectores
mientras tanto: un trial fallido cuesta un frame de fallback, no uno de sombra faltante).
Test de aceptación `the_bias_settles_the_denials`: pool de 4 con demanda de ~11 → el bias
escala hasta que `denied == 0`, se queda quieto 3 frames (sin oscilación), y con la demanda
relajada vuelve a (0,0) solo. El panel imprime el bias vigente; uno que queda alto es el pool
diciendo que es chico para la escena. `RANK_WORDS` 36→40 (bias + paciencia persistentes; el
clear por frame ahora borra SOLO el histograma).

🎉 **2026-08-23 (6) — EL POOL ASIGNA POR RANGO, NO POR ORDEN DE LLEGADA (#942).** El
diagnóstico salió del propio panel: **7 674 páginas pedidas contra un slice de 1 024** y un
estado estacionario `1022 reused · 0 new · 0 evicted` — el que agarró un slot primero lo
retiene para siempre (residente → re-marcado → edad refrescada) y 6 652 requests hacen
inanición eterna; mover la cámara producía pedidos que jamás aterrizaban. Tres dispatches
nuevos al final del marking: `plan_view` (prefix-sum del histograma de demanda por rango
contra el presupuesto del slice → rango de corte + quota + spare), `preempt_view` (desaloja
lo que el plan no financia; los residentes del rango de corte toman la quota ANTES que los
nuevos — a igual importancia, una página CON contenido le gana a una sin), `adopt_view`
(asienta lo financiado; lo demás se DENIEGA y se cuenta). El rango es el nivel, grueso
primero: el clipmap del sol (rangos 0..17) delante de toda lámpara, y dentro de cada chain lo
grueso delante de lo fino — bajo presión se pierde detalle, nunca cobertura, y el sol nunca
pierde contra una lámpara. `page_touch` ya NO asigna: la demanda se anota donde se gana el
bit y la asignación es del plan. La aritmética cierra por construcción (el plan financia
exactamente `slice` asientos y la preempción libera lo no financiado → `overflow` del
allocator = 0 SIEMPRE; toda escasez es una *denial* con rango). Dos tests de aceptación:
`the_survivors_are_the_top_ranks` (más demanda que slots → todo residente ≤ cutoff, espejo
CPU de `entry_rank`) y `a_saturated_pool_reseats_on_move` (pool saturado + cámara movida →
`claims > 0` y `preempted > 0` EN EL MISMO FRAME, no a los `max_age`). El binding 9 (libre
desde la tabla plana) se gasta en el rank state: el layout queda EN el límite downlevel de 8
storage buffers. ⚠️ #942 no hace que 7 674 entren en 1 024 — eso es el bias de resolución
(#943) y el gate por cobertura (#944).

🎉 **2026-08-23 (5) — BLUR CONFIGURABLE (#941): `shadow_softness` en RenderSettings.** El
filtro de páginas generaliza de bilineal fijo a caja Castano-class de ancho configurable en
téxeles (1 = bilineal exacto del cube path, el default; 2/3/5 con pesos de borde `frac`-clipped
y precisión sub-téxel; costo `(W+1)²` loads POR LUZ POR PÍXEL — por eso el default es sharp y
la opción ancha lleva su factura en el label). El ancho viaja en `world.w` del uniform del
raster (el shading bindea ESE buffer — una escritura sirve a ambos). Cadena completa:
RenderSettings → `shadows()` → `ShadowSettings.page_softness` → `PageSettings.softness` →
`raster.set_softness` → uniform → `inti_page_filter`, con test de alcance
(`shadow_softness_reaches_the_published_settings` — la clase de bug que shippeó
`virtual_shadows` inerte). Sin blocker search: penumbra uniforme, no contact-hardening (eso
sería PCSS, fuera de #941).

🔴 **2026-08-23 (4) — EL USER PROBÓ: el cache rinde ("muchísimo más performante") pero
`many_lights` tiene CIEN luces y el cap era 64.** El panel lo decía entero: 2.4M pares/111k
samples ≈ 22 luces por píxel — no 16. Las 36 luces en slots 64..99 caían en la rama over-cap:
121 páginas dropped, un tercio de la escena sin sombra, y cada luz SIN sombra lavando las
sombras de las vecinas (el "se borran/no se suman" del user era esto — la suma por luz del
shading es correcta; faltaban las sombras de un tercio de las luces). Fixes (`—`):
`LAMP_CULLS` 64→**256** (el presupuesto del clustering); la arena de errores se dimensiona por
las luces ACTIVAS del frame, no por el cap (256 slots sobre una escena vacía no pagan arena);
el WARN ahora distingue "luces sobre el cap" de "sin espacio" y dispara solo en la TRANSICIÓN
(con luces animadas, `pages` se movía cada frame y re-armaba el warn — 2 000 líneas idénticas);
test nuevo `a_hundred_lamps_compact_without_drops` = la forma exacta de la escena. Pendiente
del feedback del user: **blur configurable = #941** (siguiente).

🎉 **2026-08-23 (3) — LAS PÁGINAS CACHEAN SU CONTENIDO ENTRE FRAMES (#477/#866): "cached
pages are effectively free" quedó implementado.** El pool ya persistía slots; ahora persiste
el DEPTH. La 4ª palabra de la tabla volvió como **content stamp** (la generación bajo la que
se dibujó; 0 = sin contenido) y la compactación saltea toda página residente cuyo stamp
coincide: ni lista, ni expande, ni dibuja. **Murió el clear de capa entera**: el depth pass
carga la capa (`LoadOp::Load`) y limpia SOLO los rects sucios con un quad por página (compare
`Always`, reversed-Z 0). Generaciones (hash FNV, nunca 0): por NIVEL del sol (centro snapeado
— espejo exacto de `sun_centre`, verificado por test dedicado —, dirección, y el eje del ojo a
lo largo del sol porque el origen de profundidad viaja con el ojo), y por LÁMPARA (posición,
dirección, range, kind, cono — la granularidad de UE5). Invalidación por movimiento:
`instance_bounds` (#847) ya tenía esfera+hash por instancia → diff contra el frame anterior →
esferas viejas Y nuevas al `cs_invalidate`, que apaga el stamp de toda página alcanzada (por
página en el sol, por luz en lámparas; por celda = refinamiento en #866). Overflow de la lista
de movidos o del pair list ⇒ bump de scene generation = todo redibuja UNA vez, nunca stale. El
panel ahora imprime `rastered · cached ·` (criterio #477: sucias <5%). El rig lo prueba en
caliente: 2º frame ⇒ 0 listadas, 2 cacheadas, atlas intacto; caster movido ⇒ vuelven las 2 y
el redraw reproduce la escena. ⚠️ Corrección honesta: la lámpara fuera de alcance que el
commit anterior decía plantar en el rig NO estaba (un splice fallido la perdió y el assert
pasaba trivialmente) — plantada de verdad en este commit.

🎉 **2026-08-23 (2) — EL CULL DE LÁMPARAS ES UNA JERARQUÍA EN GPU (#939), y la técnica del
paper quedó completa en su parte de culling.** El user trajo el paper fundacional (Olsson et
al. 2014, *Efficient VSM for Many Lights* — el ancestro del VSM de UE5) y pidió completar la
técnica ANTES de medir. Gap analysis contra Kóoch: clustered shading ✅ (#780), selección de
resolución ✅ (mejor: por página vía `wanted`), projection maps ✅ (la expansión solo empareja
contra páginas residentes), multi-draw ✅ (un `draw_indirect`), LOD ✅ (mejor: DAG de meshlets).
Faltaban: **el cull jerárquico (§3.4/§5.2)** — hecho hoy —, el max-depth cull (→ #940), el
filtro suave (→ #941) y la caché de contenido (ya en #477/#866). Lo de hoy: `lamp_cull.wgsl`,
4 dispatches compartidos por TODAS las lámparas, **una vez por frame** (view-independent — la
segunda cámara del editor reusa los survivors): (1) pares luz×instancia (esfera de la luz vs
esfera del mesh, `mesh_bounds` del #847 subido a GPU), (2) args indirectos, (3) **la reducción
de error por grupo del #465 para todas las lámparas en UN dispatch** — arena
`[slot × group_capacity + group]`, coherencia entre siblings intacta, sin costuras en los
casters — y (4) el cull (LOD perspectivo desde la luz + range + cono) emitiendo a slices fijos
de 4096 por lámpara, counts directo a `visible_counts` (sin copia, sin bind groups por
lámpara). Murieron: los 32 `MeshletCull` por lámpara de la mañana, su loop CPU y sus bind
groups. `LAMP_CULLS` 32→64 (un slot ya no cuesta un cull; techo honesto = la arena,
`64 × group_capacity × 4 B`). El rig ahora planta además una lámpara fuera de alcance y
verifica su slice VACÍO. Pendiente nombrado en #939: ranking de slots (hoy orden de buffer).

🔴 **2026-08-23 — LAS LÁMPARAS DEJAN DE PEDIR PRESTADOS LOS SUPERVIVIENTES DEL SOL: un
cull por lámpara.** El "cero culls nuevos" de abajo era el defecto, medido en dos síntomas del
user: la sombra de una point se **desintegraba al acercar la luz al objeto** (pedía octavas
finas → buckets finos → cajas ortográficas chicas centradas en la CÁMARA → sus casters quedaban
fuera y el cull se los comía), y la spot dibujaba con **meshlets raíz** (bucket grueso → LOD del
sol grueso → la esfera facetada). Una lista de supervivientes es un LOD elegido para una VISTA;
las del sol son de las vistas del sol. El fix es la receta del cube path retirado (#777):
**un cull por luz puntual** — frustum = caja ortográfica de `2×range` centrada en la luz, LOD
**perspectivo desde el ojo de la luz** (`with_lod`, viewport `LOCAL_MAX_TEXELS`, 90° ⇒
`proj_scale_y = 1`) — y **una sola lista sirve las 6 caras y toda la cadena** porque el error
perspectivo ya escala con la distancia. No es la explosión 4848 que este diseño temía: es
`17 + lámparas` culls, cap `LAMP_CULLS = 32` (el `MAX_POINT_SHADOWS` del camino clásico; una
luz sobre el cap queda listada y contada como dropped, no silenciosa). La compactación bucketea
las locales por **slot de luz** (`chain.x + slot`), murió la 4ª palabra del "ask" (la tabla
vuelve a 3 palabras/entrada) y el binding de lights de la compactación se retiró. El rig
end-to-end ahora planta fina+gruesa y verifica que ambas caen en el bucket de SU lámpara y que
el atlas trae piso+caja del cull propio.

🎉 **2026-08-22 — LAS POINT Y SPOT LIGHTS DIBUJAN Y SE SAMPLEAN.** Cinco piezas, ninguna
útil suelta, y por eso `mark_local` reclamaba `false` hasta que estuvieron las cinco:

| pieza | qué era |
|---|---|
| **bucket = OCTAVA** | Un bucket es una densidad, no una luz ni una cadena. `page_octave` está anclada para que el nivel L del clipmap del sol caiga en el bucket L **exacto**, así una lámpara cae en listas de supervivientes que los culls del sol **ya llenaron**. Medido: una lámpara de 10 m ocupa `[0,0,0,1,2,3,4,5]`. **Cero culls nuevos, y el costo no crece con la cantidad de luces** |
| **cono en `cs_expand`** | Una página de lámpara es un frustum desde un punto, no una losa; el test esfera-caja es incorrecto a toda distancia salvo la que la caja usó |
| **`page_clip_w`** | La página era un **mapeo, no una proyección**: se dividía por vértice y el rasterizador recibía `w = 1`, así que el interior del triángulo se rellenaba con rectas entre tres esquinas divididas por separado. Wrong de forma coherente y direccional — se leía como *todas las sombras apuntando al mismo lado* |
| **`face_local`** | Rechazar un vértice por caer en otra cara **no elimina el triángulo**: el clipper interpola y dibuja una cuña en cada costura. En pantalla, una **barra recta de oclusión falsa** cruzando el pool. Ahora se proyecta sin preguntar y un punto detrás vuelve con `w` negativo |
| **el LECTOR** | `inti_point_shadow` sampleaba el cubemap pase lo que pase. 7937 pares por frame rasterizados y **nadie los leía**. Y el camino de páginas **no** se gatea con `shadow_slot`: ese slot es un índice de cubo y hay 32, que es el techo que las páginas existen para borrar |

`LOCAL_MAX_TEXELS = 2048` corta la cadena de una lámpara tres niveles — **factor 64 en las
páginas que puede direccionar**. Sin eso, 455 de 504 slots residentes eran de lámparas, al sol
le quedaban 49, y la tabla caminaba **9 tumbas por lookup**.

🔴 **2026-08-22 — EL COSTO NO ESTÁ DONDE LO BUSCÁBAMOS, Y LA TABLA HASH ES EL DEFECTO.**
Profiling de 1096 frames, con las lámparas dibujando:

```
raster + shade                   12.398 ms
  shade: compute (half rate)      6.432      ← el lector
  shade: compute                  4.011      ← el lector
shadows                           0.884      ← TODO el track: marcado, cull, compact, expand, draw
```

**El sombreado se come 10.4 ms y todo el track de sombras 0.88.** El híbrido de la expansión
—medido en **245×** de ahorro sobre 254 898 pair tests— optimizaría el 7 % del frame. No es ahí.

Lo que cambió: `inti_point_shadow` era **un `textureSampleCompareLevel`** — una instrucción de
textura con hardware dedicado. `inti_local_page_shadow` es un **walk en software**: hasta 5
niveles de cadena, cada uno con un lookup de hash abierto de hasta 32 sondeos, **por píxel y
por luz**. Se cambió una instrucción por un bucle anidado.

**Las tres fuentes del prior art coinciden en que el lookup es UNA lectura indexada:**

| fuente | qué dice |
|---|---|
| Chalmers, *More Efficient VSM for Many Lights* | *"virtual shadow maps are quite fast because they only require **a single texture lookup** in the final pass"* |
| Stephano, *Sparse Virtual Shadow Maps* | `pageTable[ivec2(floor(virtualTexCoords * numPagesXY))]` — **una indirección por píxel**, entrada de 32 bits con coordenada física + índice de pool + residencia |
| UE 5.8 | `SampleVirtualShadowMapLevel` → `VirtualToPhysicalTexel`, **un solo lookup** |

🎯 **Y el recorte de hoy destrabó la decisión que estaba bloqueada por memoria.** La tabla plana
se había descartado por costar **108 MiB** — pero ese número asumía darle a cada luz el espacio
virtual completo. Con `LOCAL_MAX_TEXELS`:

| | páginas virtuales | tabla plana |
|---|---|---|
| antes (cadena completa por lámpara) | 28 409 856 | **108 MiB** 🔴 |
| **después del floor** | **~485 000** | **~1.9 MiB** ✅ |

**Sesenta veces menos.** La razón por la que la tabla es un hash abierto desapareció, y con ella
el walk de 32 sondeos por píxel por luz.

🔴 **2026-08-22 (noche) — LAS ESCENAS SIN SOL ESTABAN ROTAS POR UN GATE MUERTO, y la spot por
tres mapeos.** Probado por el user en `roll-a-ball` (solo point lights): sombras destruidas y
las debug views de lámpara en magenta. Tres defectos (`dbf1b6c7`): (1) `record_page_raster`
retornaba temprano sin sol — comentario de ANTES de que existiera el ráster local ("their
raster is the next machine") — así que las lámparas marcaban páginas, quemaban slots del pool
y **nadie compactaba ni dibujaba nada**: el lector sampleaba el atlas viejo de otra escena.
Ahora el clipmap cae a -Y (el mismo default del marcado). (2) La **spot** era tres mapeos
distintos de una página: marcado y lector forzaban cara 0 con la UV del EJE DEL MUNDO, y el
ráster proyectaba por +X del mundo → `spot_local` en `page_table.wgsl` rota el offset para que
el eje de la spot SEA la cara 0, compartida por las cuatro pasadas como `sun_basis`
(test `a_spot_page_rotates_with_its_axis`, por las funciones del shader). (3) Las debug views
de lámpara se cegaban con `shadows_enabled` — flag de CASCADAS, 0 sin sol. Además: con páginas
activas se seguían dibujando los cubemaps y la layer de spot **que nadie samplea** (seis caras
por lámpara de costo muerto) — listas vacías bajo `virtual_pages` y el atlas fijo se libera; y
los logs INFO por frame bajaron a debug. ⚠️ Del capture del user (1073 frames, desktop): GPU
1.2 ms, CPU ~4.7, **`vkAcquireNextImageKHR` 17.0 de 24.6 ms** — el "no llega a 60" es el
swapchain, no el engine; `KOOCH_PRESENT_MODE=novsync` lo demuestra en un launch.

✅ **2026-08-22 — LA TABLA PLANA ESTÁ.** `page_table.wgsl` ya no tiene hash: el índice de la
entrada ES la página virtual, la primera palabra es `slot + 1` (0 = ausente), y el lookup del
sombreado es **una lectura indexada** — la forma de Chalmers/Stephano/UE5. Con ella murieron
los tombstones, el `sweep_view` entero, el buffer de keys (un binding menos en tres pasadas) y
los tres contadores del hash (`holes`/`probes`/`swept`). El espacio local se re-basó en el piso
— stride por lámpara **2 048 páginas contra 131 070** — y los slots de luces van acolchados de
a 64 para que agregar una luz no mueva ni una página residente. El walk del lector quedó en ≤5
niveles × una lectura cada uno (el del sol resuelve típicamente en el primero).

⚠️ Y lo que el prior art dice del ráster y todavía no tenemos: **caché de páginas entre
frames**. StraySpark: *"cached pages are effectively free"*; el juego entero de optimización es
mantenerlas cacheadas. Nuestro pool se vacía y se rellena cada frame.

🔴 **2026-08-21 — MEDIDO EN EL EDITOR: la VSM anda mal, y los números dicen por qué.** Dos
vistas alternando a 409x403, `many_lights`:

| | View | Game |
|---|---|---|
| `resident` (marcadas) | 2520 | 1666 |
| `local` (marcadas, NO dibujadas) | **2008** | 1646 |
| **páginas del SOL rasterizadas** | **40** | **20** |
| pares meshlet/página | 930 | 1064 |

**Cuatro defectos, en orden de gravedad:**

1. 🔴 **El pool se llena y desborda.** El panel lo dice solo: *"539 pages went unallocated —
   the pool is full"*, *"2048 of 2048 pool pages allocated · 100% full"*. `resident` 2520
   contra 2048 de capacidad: **472–539 páginas nunca reciben slot**, y por eso la compactación
   ve muchas menos de las marcadas.
2. 🔴 **Las luces locales se comen el pool y NADIE las dibuja.** 2008 de 2048 slots son
   locales, que el ráster todavía no soporta. Al sol —el único consumidor real— le quedan
   **40 páginas**. Por eso la escena casi no tiene sombra. **La asignación no tiene prioridad
   ni excluye lo que el ráster no puede usar**: marca, asigna y tira.
3. 🔴 **UN pool para DOS vistas.** `PageMarker`/`PageRasterizer` viven en el stage, no en la
   vista. Cada vista limpia y rellena la MISMA tabla cada frame, así que **lo que una vista
   samplea lo llenó la otra** — que es exactamente el "en una view se ve y en la otra no".
4. 🔴 **Tormenta de allocations.** Los bind groups se crean **dentro del loop de niveles**:
   17 `visible_bg` + 17 `expand_bg` por vista por frame, más los de los culls. Con dos vistas
   son >70 bind groups por frame sólo de este pase.

⚠️ **Sin explicar todavía**: 40 páginas dan 930 pares y 20 páginas dan **1064**. Menos páginas
y más pares no cierra; hay que mirar el mapeo página↔meshlet de la expansión antes de tocar
nada más.

✅ **2026-08-21 — el prior art quedó LEÍDO, y desmintió tres cosas que dábamos por ciertas.**
Del paper de Chalmers (_More Efficient Virtual Shadow Maps for Many Lights_, TVCG 2015) y del
source de UE 5.5 — `VirtualShadowMapPageAccessCommon.ush`,
`VirtualShadowMapPhysicalPageManagement.usf`, `VirtualShadowMapPageMarking.usf`,
`VirtualShadowMapBuildPerPageDrawCommands.usf`, `VirtualShadowMapPerPageDispatch.ush`:

1. **Cuando el pool se llena no se tira nada, y no hay prioridad por luz ni por nivel ni por
   distancia: es LRU puro sobre un pool que PERSISTE entre frames.** Cuatro listas de
   `MaxPhysicalPages` — `LRU`, `AVAILABLE`, `EMPTY`, `REQUESTED` — y una página no pedida
   sobrevive mientras `PhysicalPageRequestedAge <= MaxPageAgeSinceLastRequest`. Si
   `PopPhysicalPageList(AVAILABLE)` vuelve vacío, Epic sencillamente **no escribe nada** y el
   sampler cae a un nivel más basto. El overflow es degradación, no pérdida.
2. **🔴 UE5 NO hashea la tabla de páginas.** `CalcPageOffset` es aritmética plana:
   `id * VSM_PAGE_TABLE_SIZE + level_offset + x + y * dims`, **21 845 entradas = 87 KiB por
   shadow map**. Se mantiene chica porque una luz lejana **no recibe espacio virtual completo**:
   `VSM_MAX_SINGLE_PAGE_SHADOW_MAPS` son 8192 mapas de UNA entrada. Nuestros 108 MiB salían de
   asumir el espacio completo para las 101 luces — una decisión nuestra, no una ley.
3. **No se marcan páginas para consumidores que no existen**: `PruneLightGridCS` reescribe la
   light grid dejando sólo luces con `VirtualShadowMapId` y manda las distantes al final.
   Marcar y dibujar leen **la misma lista**.
4. **No hay pares (página, meshlet) ni loop de niveles en la CPU.** `FPerPageDispatchSetup` usa
   `DispatchThreadId.y` como índice en un buffer `VirtualShadowMapIds` — todos los mapas y todos
   los niveles en **un dispatch**. Y `CullPerPageDrawCommandsCs` emite **un comando por
   (instancia, nivel)** con un RECT de páginas, no uno por página.

⚖️ El source de UE está bajo su EULA: se estudia el **diseño**, no se copia una línea a Kóoch.

⏭️ **NEXT — y va PRIMERO: leer cómo lo hace quien lo hizo bien.** Los cuatro defectos de
arriba son de **arquitectura**, no de aritmética, y ya se demostró que este track adivina mal
cuando diseña sin fuente. Antes de tocar código:

| Fuente | Qué contesta |
|---|---|
| **Olsson, Sintorn, Kämpe et al. (Chalmers), _Efficient Virtual Shadow Maps for Many Lights_** | 🎯 **El caso exacto**: muchas luces + shading clusterizado + VSM. De ahí ya salió el marcado desde las view samples. Contesta el defecto 2 (cómo se reparte un pool entre muchas luces) y probablemente el 1 |
| **UE5: `VirtualShadowMapArray`, `VirtualShadowMapCacheManager`, `VirtualShadowMapPageManagement.usf`** | 🎯 Un pool para **N vistas** con tabla por vista → defecto 3. Y su política cuando el pool se llena → defecto 1 |
| **Nanite: la ruta VSM del rasterizador** | Cómo arman la lista de instancias `(meshlet, página)` → el misterio de los pares |

**Preguntas concretas a contestar con SOURCE, no con blog:**
1. Cuando el pool se llena, ¿qué páginas se tiran y con qué criterio? ¿Hay prioridad por luz,
   por nivel, por distancia?
2. ¿Se marcan páginas para luces que el ráster no va a dibujar, o el marcado ya sabe qué
   consumidores existen?
3. ¿La tabla es **por vista** o hay una sola con la vista adentro de la clave?
4. ¿Cómo se construye la lista de pares sin un bind group por nivel por vista por frame?

✅ **(b) y (c) hechos: la VISTA entra en la clave y los bind groups salieron del loop.**

- **El id de página lleva la cámara arriba**: `page = view * view_span + light * stride + …`,
  que es el `VirtualShadowMapId` de UE con un multiply en lugar de una tabla por id. El hash
  hace que agrandar el espacio de direcciones salga gratis.
- **La tabla ya no se vacía con `clear_buffer`**: un pase `clear_view` borra **sólo** las
  entradas de la cámara que va a marcar. Tenía que ser un pase porque el ráster está fusionado
  con el shading — una vista samplea un atlas de un frame atrás, así que vaciar la tabla entera
  al principio del frame deja a la segunda cámara leyendo lo que la primera acababa de borrar.
  **Ése era el "en una view se ve y en la otra no".**
- **El pool se SLICEA, no se comparte**: cada cámara tiene su rebanada y el atlas pasó a ser un
  **array con una capa por vista** — una capa es un attachment que una cámara limpia sola. Dos
  viewports cuestan lo que costaba uno: la rebanada es `pages / views` redondeado al cuadrado.
- **Los bind groups se construyen UNA vez** y se invalidan comparando los buffers que hay
  detrás. Lo que cambia por vista y por nivel viaja como **dynamic offset**. Eran 34 por cámara
  por frame sólo en el loop de niveles.
- 🔴 **El uniform del ráster tiene una rebanada por cámara.** `Queue::write_buffer` no está
  ordenado contra el encoder (#853): escribir el mismo rango dos veces en un frame le da a AMBOS
  pases el segundo valor — o sea que la cámara A rasterizaba su clipmap **con el ojo de la
  cámara B**.
- **El binding del lector se hace ANTES del pase fusionado**, no después: el pase fusionado ES
  el shading.

⚠️ **Todavía sin verificar en pantalla.** Los tests cubren el mecanismo — uno de ellos falla con
el comportamiento viejo y pasa con el nuevo — pero nadie miró un frame con dos viewports.

✅ **(a) hecho: el marcado ya no le reclama pool a las luces locales.** Medido en el editor con
las dos vistas: **991 y 1004 de los 1024 slots de cada cámara eran locales**, y al sol —el único
consumidor que el ráster tiene— le quedaban **33 y 20 páginas**. El pool se declaraba 100% lleno
sin producir sombra. Las páginas locales **se siguen marcando**, porque lo que costarían cien
luces proyectando es la medición que justifica todo este track; simplemente no gastan pool hasta
que algo las rasterice. Epic dice la misma regla como pase: `PruneLightGridCS` poda la light grid
a las luces que **tienen** un shadow map **antes** de que nada marque.

🔴 **2026-08-21 — el ráster de páginas culleaba EXACTAMENTE la geometría que proyecta.** Lo
intuyó el user mirando el editor: *"creo que las caras de las meshlets pueden estar invertidas"*.
Tenía razón, y no eran las meshlets. Entre un triángulo del mundo y el clip space de una página
hay **dos flips**, y sólo uno de ellos entra en el mapa 2D por el que el rasterizador decide el
winding:

- `sun_basis` devuelve `(s, u, f)` con `u = cross(s, f)` → determinante **-1**, base
  **zurda**, a diferencia del `look_to_rh` de las cascadas.
- `page_clip` niega la Y porque el rect de una página está en filas de téxeles —que van hacia
  abajo— y el clip space va hacia arriba. **El lector está de acuerdo con ese flip**, así que
  sacarlo espejaría todas las sombras en lugar de arreglarlas.

Medido en la GPU a través de esas mismas funciones: un triángulo que mira al sol sale con área
con signo **-0.25**, o sea `Cw`. El pipeline declaraba `Ccw`, así que `cull_mode: Back` tiraba
todas las caras que proyectan y dejaba **la cáscara trasera de cada malla cerrada**. Sombras
como manchas con agujeros que cambiaban de forma con el nivel del clipmap — y con él, con el LOD
del meshlet. De ahí el *"se rompe en base al meshlet"*.

⚠️ **Y el cull de meshlets empeoraba el cuadro**: `camera_in_cone` con la "cámara" en
`eye - sol * SUN_SPAN` conserva los meshlets que **miran al sol**, y el ráster tiraba los
triángulos que miran al sol. Lo que sobrevivía era un subconjunto casi arbitrario, agrupado por
meshlet.

⏭️ Falta: (d) el misterio de los pares, (e) re-medir. Y encima de todo eso, **lo que el prior art
dice que es el mecanismo y no una optimización: persistencia entre frames con LRU**, más una
clase "una sola página" para las luces lejanas.

**Phase 3 — the consumers, on top of #866 and not before.**

| | | Why it waits for the pool |
|---|---|---|
| **#450** | surfel / surface radiance cache | 🎯 **The only item on this board that changes the SHAPE of the cost** — from *pixels × lights every frame* to *cache texels × lights at a low rate*. A meshlet is a natural cache page, which is an advantage Lumen's cards and SEED's surfels both work around |
| **#477** | virtual shadow maps | Its justification is **scale and memory** — four cubes for a hundred lights, 152 MiB standing — **not frame time**, because shadows total 1.153 ms. Any summary promising milliseconds here is wrong |
| ~~#841 / #849~~ | which four lights get a cube | **Superseded if #866 lands**: pages allocated by need retire the question of slots handed out by rank |

**Phase 4 — the additive features, once there is headroom to spend.** #254 (post + auto
exposure), #771 / #248 (atmosphere), #731 (clouds, which cost 39 ms as written). Every one of
these *adds* to a frame that is 1.8× over budget, so they are gated on Phase 1 buying
something.

🎯 **…with one exception, and it is a prerequisite rather than a feature: auto exposure.** Every
temporal resolve — ours and every vendor backend — takes an `exposure` input and does its
history rejection in a perceptual space, and this engine's radiance is in the **hundreds**.
Feeding an unexposed image to a reconstruction is feeding it noise. So the exposure half of
#254 belongs *inside* Phase 1, which is the same conclusion already recorded about anything
that compresses range.

🔴 **The gate on the whole plan.** 24.8 ms of GPU against 13.9, in a scene built to break the
engine with a hundred shadow-casting lights. **A real level has never been measured**, and if
Phase 0 does not close most of the gap, the honest next move is to measure one before
committing to Phase 1's weeks.

#### A — the instrument, before the optimisation

`/run/user/1000/gamescope.*/stats.pipe` emits `fps=` and `focus=` and needs nothing
instrumented. **That is the outside measurement this roadmap has been asking for since the
doubled-frame section below**, and the first thing it did was disagree with us.

```
gamescope stats pipe   50-70 fps        (20.0 ms)
our capture            p50 41.4 ms      (24 fps)
gpu_busy_percent       93-99 %
GPU work per frame     ~31 ms, flat in EVERY decile
```

`gpu_busy` at 95 % with 31 ms of GPU work closes with ~32 fps, so the suspect is gamescope
— but *suspect* is not *refuted*, and this file has twice recorded a doubled-frame
hypothesis that later fell. The tiebreaker is `KOOCH_FRAME_METRICS=log` as a Steam launch
option: wall clock between frame starts, measured inside the game, no transport.

⚠️ `focus=steam` in that pipe means the game is **not** in front and the number is void.
Check it before believing anything — an unfocused window is how this project got a 5×
better and entirely false reading once already.

🟢 Re-confirmed while we were there: **the profiler client costs nothing.** Same run,
50.6 / 49.6 / 50.2 fps before, during and after attaching it. The "846 frames in 30 s" is
the transport dropping frames, not the game slowing down.

#### B — six milliseconds already being spent that buy nothing yet

The device frame, `many_lights.scene`, v0.2.41:

```
raster + shade                 28.6 ms
├─ shade: compute (half rate)  15.4      ← half the GPU frame
├─ taa                          3.7      ← new since 2026-08-14
├─ motion vectors               2.6      ← new
├─ shade: upsample              1.7
├─ tonemap                      0.8
└─ (self)                       4.2
shadows 0.82 · blit 0.55 · sky 0.31 · cluster grid 0.21
```

**TAA + motion vectors are 6.4 ms, 21 % of the GPU frame**, on a device already 2.2× over
budget. They are worth that only if something needs temporal averaging, and the one thing
that would have — #826's sampling — is **removed**: its noise was per froxel, and a
temporal resolve cannot integrate a 75x80 px block that repaints itself. The 6.4 ms
the resolve costs currently buys anti-aliasing and nothing else.

#### Measured, and not a 5×

Against the like-for-like capture of 2026-08-14, same scene:

| | 2026-08-14 | 2026-08-16 |
|---|---|---|
| frame median | 39.31 ms | ~40 ms |
| `raster + shade` | 31.41 | **28.7** |
| shadows / blit / sky / cluster grid | 0.699 / 0.479 / 0.268 / 0.153 | 0.824 / 0.551 / 0.311 / 0.209 |

⚠️ The four passes nothing here touches are all **16-37 % higher**, so this run's machine
was slower — 11.6 W, `sclk` 1141 of 2900, 58 °C, which is **power-limited and not
thermal**. Against that drift `raster + shade` still came down.

🔴 And the frame is **not** five times better. The 72.17 ms on record is a *different
scene*; this build's `main_scene` is `many_lights.scene`. Check `project.kooch` before
comparing two captures — that mistake was made and retracted the same day.

🔴 The doubled frame **survives everything shipped**: `frame/GPU` p0 1.05 → p90 2.29, with
the GPU work flat at ~31 ms across all of them. Slow frames do exactly the work fast ones
do.

### ❌ The doubled frame is not the swapchain, 2026-08-15

Two explanations were carried for weeks, one of them written into
`gpu/context.rs` as fact. Three 30-second captures on the OneXFly, one
binary, one variable changed each, killed both:

| | latency 2 | latency 3 | `novsync` |
|---|---|---|---|
| frame/GPU p80 | 1.98 | 1.99 | 1.94 |
| frame/GPU p90 | 2.49 | 2.50 | 2.21 |
| `vkAcquireNextImageKHR` ms/frame | 35.209 | 33.646 | 37.162 |

A third swapchain image does not move the ratio by a hundredth. Neither
does leaving FIFO.

🔴 **An acquire of ~35 ms against a GPU of ~35 ms is not a defect.**
Being GPU-bound means the CPU waits somewhere, and `get_current_texture`
is where. Reading that number as the symptom is what sent two sessions
after the wrong thing. What is genuinely unexplained is the **tail**
alone: frames where the wait grows by 50 ms while our GPU work grows
by 2.

⚠️ **A present mode is close to decorative when a compositor owns the
display.** These captures run under gamescope, which composites on the
same GPU, on its own schedule, and is invisible to our scopes — they
time our passes and nothing else. `novsync` turns off *our* vsync, not
its. **No environment variable on this side is going to find what is
left**; the next measurement is gamescope's own statistics, or a run
without it.

#### And what the tail was hiding

```text
GPU:          ~35 ms
budget:        13.9 ms
```

**2.5x over, with a still camera and at half shading rate.** If the tail
vanished entirely the frame would still miss by more than double.
`shade: compute (half rate)` alone is 19.7–22.8 ms across the three
captures — more than the whole frame is allowed.

The tail is a mystery in 30 % of frames. The shading is 60 % of every
one of them, and the largest thing still untouched inside it is the
contact-shadow march: 16 steps per light that reaches, ~14 lights, and
no cap anywhere. The atlas caps projected shadows at 4+4; the march caps
nothing.

`contact_shadow_steps: 0` was the single-variable test that had never
been run, and the reason it had not is that the value only existed in
the settings asset — reaching it meant repacking and copying a build,
which changes two things at once. It is now
**`KOOCH_CONTACT_SHADOW_STEPS=<count>`**, a launch option like the five
before it, so `16 → 8 → 4 → 0` runs against one binary. The count and
not a switch: the ladder separates the taps from the setup, an off
switch only says whether the whole thing is free.

### 🔴 A range compressor needs the exposure, and this engine's radiance is nowhere near 1

The temporal resolve shipped looking like a broken toon shader: a dark
rim on every silhouette and iso-luminance contours sweeping the floor.
It was reported from a screenshot before any assertion here caught it,
and the assertions could not have caught it — every number they produce
is a magnitude, and this artifact is *signed*. What found it was dumping
the difference between the resolved and unresolved frames amplified
about mid grey, which is now `dump_frames` in `tests/temporal_aa.rs`.

The cause is one line, and it generalises past TAA. The resolve blends
in a range-compressed space, `c / (max(c) + 1)`, so that one firefly
cannot drag a neighbourhood. That operator has all of its resolution
between 0 and about 4, and it is written against a scene whose radiance
sits near 1.

**Ours does not.** Exposure is applied downstream in the tonemap pass,
and the scene is blown out besides (#254), so lit surfaces reach the
resolve at radiance in the hundreds. Compressed, every one of them lands
between 0.998 and 0.9999 — the whole image inside a thousandth of the
operator's range. Blend there, expand with the inverse, and what comes
back is posterised: flat bands with hard boundaries, and a rim wherever
two bands meet.

Multiplying by the exposure before compressing, and dividing after, is
the whole fix. Every measurement moved the right way at once, which is
what separates a fix from a tuning:

| | before | after |
|---|---|---|
| still scene, frame-to-frame | 0.172 → 0.181, **growing** | 0.099 → 0.095, settling |
| pan, distance from the unresolved frame | 1.506 | **1.058** |
| after stopping, pan history vs fresh | 0.450 | **0.324** |

⚠️ The rule to carry: **anything in this engine that compresses range —
a resolve, a bloom threshold, a firefly clamp — has to see the exposure
first.** The renderer's linear values are not display-referred and are
not near 1, and every operator borrowed from a renderer whose values are
will misbehave in a way that looks like a shading bug rather than a
scaling one.

⚠️ **Still missing from the port:** Bevy `#[require]`s `MipBias`
alongside its TAA, and we have none. Jittered accumulation reconstructs
detail finer than one frame carries, so textures want a negative LOD
bias to match. Open.

### 🔴 A temporal pass is not free to port, and the copy cost more than the write

#481 was a port, per the standing rule, and the port still had to be
measured against the thing it was ported into. Two of upstream's choices
are wrong here, and neither announced itself — both render a plausible
image.

**The history holds linear radiance, not the compressed value.** Bevy
stores the range-compressed colour; read back through the inverse
operator, fp16's half-thousandth of resolution near 1 is multiplied by
`1/(1-t)²`. Frame-to-frame change of the final image on a still scene,
over twenty frames: **0.09 storing linear against 0.85 storing
compressed.** It is also one texture instead of two.

**The variance clip stays at one sigma, and nearly did not.** It
shipped at two for a day, on the strength of squared gradient summed
over the *whole* image — a number a lit floor's falloff dominates, and
which therefore reported upstream's width as making the frame worse.
Masked to the pixels that actually are edges, the ranking inverts: one
sigma leaves **0.38** of the unresolved frame's edge energy where two
leaves 0.62.

The rule that failed was not the arithmetic. Widening a variance clip is
an **anti-ghosting** parameter, and it was being tuned against a scene
that never moved — a scene with no ghosting in it. Anything that only
appears in motion has to be measured in motion, and
`tests/temporal_motion.rs` exists because this did not.

⚠️ One sigma keeps a period-eight shimmer on a still scene that two
removes: **0.18 of a level per channel per frame, and not decaying**,
against 0.08 and settling. That is the clip firing on ~11 % of the
pixels of a scene with nothing moving in it, and it is upstream's
behaviour at upstream's width. Open, measured, and not worth trading
ghosting for.

**And the metric was wrong twice before it was right.** Counting
"intermediate" pixels gave 21595 against 21337 — a lit floor is already
a gradient. Summing squared gradients over the whole image gave 124
against 123, because the lighting falloff carries the total and the
resolve rightly leaves it alone. Only masking to the strongest edges of
the *unresolved* frame separated the effect from the scene: 0.62 on the
top percent, 0.46 on the top tenth of a percent.

🔴 **And it ships OFF, in the asset as well as in the engine.** It went
in with `compute_shading` and `temporal_aa` both defaulting to true in
`.rendersettings`, reasoning that a project with a settings asset has an
author who can see the result. What happened is that every existing
project — whose file predates these fields and therefore takes every one
of their defaults — changed shading path *and* gained a temporal resolve
in one build. Two variables at once is not a change anybody can bisect,
and the first report was "you broke the whole render". A serde default
is not a recommendation; it is what an old file silently becomes.

⚠️ **What TAA does not fix, and it is the thing that started this.**
The froxel sampler's choice (#826) was seeded on the cluster index, not
the frame, so its noise was the same every frame and an average had
nothing to average. Adding the frame term was tried on 2026-08-16 and
made it worse, not better: the choice is per FROXEL, so advancing it
repaints a ~75x80 px block in a new colour every frame, and no resolve
integrates a block that size. The sampler is removed; see its row in
the table above.

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

### Two shading paths, and what "the same image" turned out to mean

#824 ships the compute path beside the fragment one rather than in place
of it, switched by `KOOCH_COMPUTE_SHADING`. Both are built every run, so
the A/B is one environment variable on the handheld and not a rebuild —
the same shape `KOOCH_CLUSTERING` has.

Everything above the entry point is the identical composed shader: the
same reconstruction, the same BRDF, the same lights in the same order.
What changes is that a workgroup owns a 16×16 tile, reduces its threads'
froxels to one block of cells, copies that block's light indices into
`var<workgroup>` memory, and every thread then walks its own cell's run
out of shared memory.

🔴 **The two paths cannot be byte-identical, and the reason is not this
issue.** The fragment path's colour reaches the `Rgba8Unorm` target
through the ROP; the compute path's through `textureStore`. The two
f32→unorm8 conversions are not required to round the same way in the
last bit, and they do not: with clustering switched off — which makes
the compute pass call the *same* `inti_shade` on the *same* data —
about 8 % of pixels still land one unit apart in one channel, maximum
delta exactly 1.

So parity is asserted as the two things that are actually provable:
**coverage is exact** (alpha is 0 or 255, never arithmetic, so a pixel
lit by the wrong path says so), and **colour agrees to within one unit**.
Every failure this issue can have — a light missing from the cache,
counted twice, or a run read at the wrong offset — moves radiance by far
more than 1/255. Deliberately breaking the tile loop moves it by 94.

⚠️ **A cap and a fallback.** A tile at a silhouette spans many z-slices,
so its block of froxels can be large. Past `MAX_TILE_CELLS` or
`MAX_TILE_LIGHTS` the tile shades straight from the storage buffer
instead — slower and correct, which is the right way round.

### Half rate, and the guide that is not depth

#825 reduces the *shading* rate without touching the raster rate:
geometry, depth and the visibility buffer stay at full resolution and
only the light evaluation runs at one sample per 2×2 quad.
`KOOCH_SHADING_RATE=half`, live and rebuild-free, the same shape the
other knobs have.

That decoupling is only possible because #824 moved shading out of the
raster's fragment shader. While it lived there, its resolution *was* the
raster's — and this would have been "render the game smaller", which is
a different thing that already exists (#481 / #536, and they compose
with this rather than replacing it).

🔴 **The upsample's guide is the visibility buffer, not depth.** The
textbook bilateral upsample compares depth, because a forward or
deferred renderer has nothing better. This one does: the vbuf already
stores which meshlet won each pixel, so "same surface" is an integer
compare instead of a depth epsilon somebody tunes per scene. Two
surfaces a millimetre apart at a grazing angle — the case a threshold
gets wrong in both directions — are simply different slots.

⚠️ **Two invariants make the reconstruction hole-free, and both are the
kind that only hold if they are designed for.** The shaded sample of a
quad is chosen from the *vbuf alone* — the first covered pixel, in a
fixed order — never from the dispatch's own material: every material
dispatch runs over every sample, so a material-dependent choice would
have two dispatches writing the same texel. And because the choice is
the first *covered* pixel, a covered screen pixel's own quad was always
shaded, and that sample is always one of the four the upsample
considers. There is no covered pixel anywhere with nothing to read.

The thresholds in `half_rate_shading.rs` were **measured by breaking the
shader on purpose**, not chosen: shifting the reconstruction offset by a
quarter texel moves the mean 1.01 → 4.29/765, and ignoring the surface
guide moves the wall's 99.9th percentile 17 → 141 while barely touching
its mean. The second is why that test asserts a percentile — silhouette
pixels are a rounding error in an average, which is exactly where a
broken guide hides.

### 🔴 The froxel flicker was never about froxels

`KOOCH_LIGHT_LIMIT` shimmered on the device, and the write-up said what
everyone assumes: a pixel crossing a cell boundary changes which lights
it evaluates. #826's temporal-stability test rendered the same
unchanged view twice and refuted it.

| | pixels changed between two identical frames | worst channel |
|---|---|---|
| walking every light | 1 | 1 |
| `KOOCH_LIGHT_LIMIT=2` | 10 098 | 164 |
| sampling, first draft | 7 075 | 83 |

Nothing moves and a third of the covered pixels change. The cause is
`cluster_raster.wgsl::write_index`: a light claims its slot in the
cell's run with `atomicAdd`, so the run's order is whichever thread
arrived first — **different every frame**. "The first two of the list"
is a different pair of lights each time.

Which makes it a trap for anything that reads the run's *order*, and
the first draft of the sampler walked into it: stratifying the
cumulative weight is one pass and one random number, and a cumulative
walk is an order. The shipped version keys each light's draw on its
**global index** instead — an argmin over independent per-light keys,
so permuting the run cannot change the winner. `-log(u) / w`, smallest
wins, which picks light *i* with probability `w_i / w_sum`.

⚠️ **It costs `K + 1` walks of the weights instead of two**, and that is
affordable only because the weight comes out of workgroup memory. Which
is where #824 stopped one step short: it cached the tile's light
*indices*, four bytes each, and every pixel still fetched the whole
80-byte `IntiLight` for every light in its froxel. Fifteen of those is
1.2 KB per pixel — #824 removed the four bytes and left the 1200, which
is why it bought 6.6 %. #826 puts the twenty bytes a weight needs in
`var<workgroup>`, read once per tile.

⚠️ **A floor in the convergence, recorded rather than fixed.** Past
about eight samples the error stops falling: ~5/255 and 1.2 % dark.
A light whose cheap diffuse weight underestimates its specular
contribution is picked rarely and scaled by a large `1/w` when it is;
the spike clips against the tonemap and an 8-bit target, and clipping
only ever loses energy. The fixes — a better weight, or a clamped ratio
— trade one bias for another, and neither is worth choosing before the
device says how much of it is visible at the two to four samples a frame
would actually ship.

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
| **Godot SDFGI** | Also indirect, so the same objection — and the directional light it would supposedly relieve is 1 of ~15 lights per pixel with 0.7 ms of shadows. Godot's docs call it "still too slow for older integrated graphics"; 6 cascades to 4 saves 0.5 ms *on a GTX 1080 at 1440p*, and there is an open proposal about stuttering when the camera moves fast — the exact condition our frame collapses under |
| **An octree instead of the uniform grid** | The lookup is not what costs. `inti_cluster_of` is four arithmetic ops and the whole grid build is 0.153 ms of 31. A tree replaces that with one **dependent** memory read per level — pointer chasing, which is the access pattern left standing after ALU was ruled out. It would worsen the measured bottleneck, and the grid is already adaptive where it matters: logarithmic z-slices, and perspective widening the cells with distance |

⚠️ **Cascades by distance are already here.** The z-slices are
logarithmic and perspective widens the froxels with distance, so the
clipmap structure an SDFGI-style scheme would add is built in, in all
three dimensions. What is missing is *sampling* the grid instead of
walking it (#826) — not a better grid. And cascades would save nothing
on `many_lights.scene` regardless: all 100 lights sit within 18 m, with
no far field to fuse.

⚠️ **Voxel injection is the exception worth naming.** VoxelGI does two
things: it injects lights into a grid, and it cone-traces through that
grid for bounces. The first half is a pixel taking one sample instead of
walking fifteen lights — useful. The second half is what makes it
expensive, and it buys bounces nobody asked for. #826's third option is
that first half alone.

### 🔴 Three suspects down, and the pixel count still standing

Each of these made the shading cheaper along one axis, and none of them
was the bottleneck:

| what was made cheaper | how much it bought |
|---|---|
| the arithmetic per light (#821 — GGX, Smith, Fresnel, multiscatter, all of it deleted) | **10 %** |
| reading the lights (#824 — 15 storage fetches per pixel → 15 per tile) | **6.6 %** |
| the grid's over-listing (#820 — measured at 24 %, not the 2.9× estimated) | nothing to win |

⚠️ **The variable none of them moved is the *number* of lights a pixel
evaluates.** Twelve to fifteen genuinely reach the surface (#820), so
clustering cannot remove them by definition — only sampling can (#826).

### 🔴 The straight line, and why both #825 and #826 get built

`KOOCH_LIGHT_LIMIT=n` caps how many of a froxel's lights a pixel walks —
in both shading paths, so an A/B between them stays one. Three runs on
the OneXFly through Steam → gamescope:

| run | lights/pixel | `shade` | raster self | frame |
|---|---|---|---|---|
| baseline (559 frames) | ~15 | 30.433 | 5.437 | 42.38 |
| A, `limit=4` (789) | 4 | 15.368 | 3.647 | 21.95 |
| B, `limit=1` (1071) | 1 | 12.178 | 3.440 | 19.34 |

⚠️ **The baseline run is not trustworthy and A/B are.** `sky` reads
0.796 against 0.251 and `blit` 1.057 against 0.447 in the baseline —
passes that do not touch a light, inflated 2-3×, so something else had
the GPU that run. A and B agree within 2 % on every one of those
controls, which is what makes the fit below a measurement rather than
three points through noise:

```
shade = 11.11 ms + 1.06 ms per light
```

**There is an ~11 ms per-pixel floor that is not the lights.** That
floor is what #825 halves and what #826 cannot touch; the 1.06 per light
is what #826 removes and what #825 only halves. Neither closes the
budget:

| | shading | frame |
|---|---|---|
| #825 alone | ~15.2 | ~27 |
| #826 alone | ~14.3 | ~19 |
| **both** | **~7.1** | **~12** ✅ |

⚠️ **The cap also produced the artifact #826 has to solve.** With
`limit` on, froxels flicker: a pixel crossing a cell boundary changes
which lights it evaluates, and the change is visible because nothing
carries state across frames. That is why every real sampling scheme
carries temporal reservoirs, and it is now a requirement written into
#826 rather than a surprise waiting inside it.

### 🔴 A weight costs a fifth of an evaluation, so the choice cannot live in the pixel

#826 shipped choosing per pixel and the device priced it, on the
OneXFly, at half rate, with everything else held still:

| samples | `shade: compute` | weights per pixel |
|---|---|---|
| 0 (walk all 15) | 12.624 ms | 0 |
| 2 | **10.482 ms** | 45 |
| 4 | 16.837 ms | 75 |

**Non-monotonic**, which is the whole finding. Solving those three for
the cost of one weight against one full light evaluation gives **0.196**
— a fifth, not the fifteenth the design assumed. At that ratio the
`(K+1) × 15` weights a pixel walks cost more than the twelve evaluations
they remove as soon as K reaches 4, and the technique appears to fail
while being perfectly correct.

The estimator was in the wrong loop. It now runs **once per froxel**,
cooperatively: one thread per (cell, stratum), at most 16 × 8 of a
tile's 256, each walking its cell's run once. What reaches the pixel is
a list of picks and their scales, so shading costs `picks` evaluations
and **no weights at all**.

This is what HypeHype's Stratified Tile-Based Lighting does (SIGGRAPH
2025), minus a level: their two-level scheme exists because they have no
cluster grid, and #780 already reduced 100 lights to ~15. Their small
tile is 16 px, which is this workgroup exactly.

⚠️ **It costs image quality and the numbers say so** — mean |Δ| against
the full walk went 8.71 → 24.33 at one sample and 5.42 → 7.53 at eight,
because one choice now serves 256 pixels instead of being averaged away.
The error also stops being per-pixel noise and becomes a discontinuity
at froxel boundaries, which for an engine with no temporal pass is the
better artefact and is chosen deliberately. **The device decides whether
the exchange was worth it. Nothing here says it was.**

Two invariants came out of it, both from the device rather than from
theory:

- **A light with a shadow map is never sampled.** A shadow is binary and
  high-contrast; a caster a tile declines to pick reads as a shadow that
  blinks, not as a slightly wrong estimate. There are at most 8 in a
  scene, so the rule is free.
- **Every light gets a floor under its probability.** A froxel is a
  volume, so a light whose range cuts through it can score zero at the
  representative point and still reach real pixels — the one failure the
  estimator cannot absorb, because a light that is never picked is never
  divided back up.

The other number that has survived every experiment is the pixel count:
the frame falls 5.2× with internal resolution, and #824 measured that
shading is 90 % of the pass the resolution scales.

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

## What gets attacked next — the order needs redeciding, 2026-08-20

✅ **Item 1 is done and it retired its own premise.** The 11 ms floor
experiment ran (#912, `KOOCH_SHADING_PAD`), and the floor it was built to
decompose does not exist any more — the whole shading pass is 3.272 ms.
What it did measure is above: **178 µs per full-screen sweep, one sweep
per material in the project.**

🔴 **And the budget is met**, which changes what this list is for. The
gate was *"a feature that cannot fit in what is left of 13.9 ms is not a
feature this engine has"*. There are now **4.2 ms** left of the 13.9, and
the GPU is at 9.7. Everything below is measured against that instead of
against a 2.9× overrun.

The measured sizes of the remaining candidates, all from the 2026-08-20
capture, so the order can be decided on numbers rather than on age:

| candidate | what it costs today | what it would buy |
|---|---|---|
| **Compact the shading dispatch** | 0.71 ms at 3 materials | 0.71 ms now, **3.7 ms at 20 materials** — the only item that scales with the game rather than the frame |
| ~~**#886 Arm ASR**~~ | — | ❌ **Closed 2026-08-20 with a verdict, read from the source.** It is FSR **2.2.2** when 3.1 is already transliterated, it removes none of FSR 2's eight passes, and its one structural change — compute to fragment — is an optimisation for **tile-based** GPUs that does not transfer to RDNA. Arm's own +53 % claim lands it near 7-8 ms against SGSR 2's 2.062. What survives is its optimisation list, which applies to **our** FSR 3.1 |
| **#477 Virtual Shadow Maps** | `shadows` is **0.549 ms of 9.7** | not a frame-time argument — planet-scale sun shadows are a product requirement no measurement retires |
| **Fuse `motion vectors`** | **0.553 ms**, was 2.6 when it was written up | ≤0.5 ms, against +4 B/px on a bandwidth-bound part |
| **TAA `textureGather`** | `taa` **does not run** under the preset that meets the budget | nothing, until somebody configures TAA and measures it |

### What Arm ASR left behind, which is the part worth keeping

Its shader quality presets are a list of named cuts, and two of them
point at passes measured on 2026-08-20:

| Arm's optimisation | against what of ours |
|---|---|
| `DISABLE_LUMA_INSTABILITY` | the `luma instability` pass, **0.571 ms** |
| `TONEMAPPED_RGB_PREPARED_INPUT_COLOR` — `R8G8B8A8_Unorm`, no YCoCg | the largest intermediate, and `accumulate` is **9.122 of 11.355** |
| `UPSCALING_LANCZOS_5TAP`, `REPROJECT_CATMULL_5TAP` | tap counts inside `accumulate` |
| depth-clip packed into the reactive mask | one target and one read fewer |

Standard ideas rather than Arm's inventions, so applying them carries no
attribution obligation. **It opens no work on its own**: FSR 3.1 is a
desktop technique and 11 ms on a desktop is not a problem. Written down
for whoever wants to bring it down.

⚠️ The previous order put #477 second and #886 third on the strength of
numbers taken when the pass was 30 ms. Both moved. **The user decides the
order; this table exists so the decision is made on today's numbers.**

---

## ❌ The 11 ms floor does not exist any more — measured 2026-08-20

Area 2 of #885 audited all 58 shaders (12 014 lines) and produced a
framing built on this fit:

```
shade = 11.11 ms + 1.06 ms per light        ← a build this engine no longer is
```

#821, #824, #820 and #826 all attacked the slope; the 11.11 ms intercept
was 37 % of the pass and had never been broken down. So an instrument was
built to decompose it (#912) and taken to the device.

🔴 **The device answered a different question, because the premise was
stale. Today's whole shading pass is 3.272 ms.** The 11 ms floor cannot
be decomposed because it no longer fits inside the thing it was a floor
of — between `render_scale: 50`, #824, #825, #820, #845 and #881 the pass
fell by more than 6×, and nobody re-measured the intercept before
designing an experiment around it.

⚠️ **The lesson is procedural and it is the expensive kind.** A number
copied from an earlier section of this file was treated as current
because it was the most recent one written down. **Re-measure the
baseline before designing an experiment around it**, especially when the
number is weeks old and the intervening work was aimed at it.

### What the instrument measured instead, and it is worth keeping

`KOOCH_SHADING_PAD=n` appends *n* full-screen shading sweeps whose
`material_id` matches no instance. Every store in
`material_pbr_compute.wgsl` is inside a branch that never fires for them,
so the frame is **bit-identical** — verified by MD5 over the rendered
pixels of both shading paths at pad 0, 7 and 250 — and the only variable
between two runs is how many times the screen is swept.

**Same session, same scene, same upscaler, 60 s each, both captures green
on `--over-time`:**

| | baseline | `pad=252` | Δ |
|---|---|---|---|
| **`shade: compute (half rate)`** | **3.272** | **48.152** | **+44.88** |
| sgsr2 | 2.062 | 1.699 | −0.36 |
| tonemap | 0.566 | 0.584 | +0.02 |
| shadows | 0.549 | 0.603 | +0.05 |
| blit | 0.424 | 0.453 | +0.03 |
| cluster grid | 0.136 | 0.112 | −0.02 |

No control moves more than 0.16 ms against a signal of 44.88 — **280:1**.

```
44.88 ms / 252 sweeps  =  178 µs per idle full-screen sweep
```

⚠️ **At 1920x1080.** A sweep is a fixed dispatch cost plus per-pixel
work — the desktop decomposition below splits them — so the number is
smaller at a lower resolution and the per-material bill has to be quoted
with one.

The same measurement on a 9070 XT reads **1.98 µs**, a ratio of 90×, and
the desktop run predicted the device to within that. It also decomposes:
at `KOOCH_SHADING_RATE=full` the desktop cost is 3.33 µs rather than
4×1.98, so a sweep is **1.53 µs of fixed dispatch cost plus 0.45 µs of
per-pixel work** at 320×180 — mostly the command processor, not the
threads.

### 🎯 What that costs, and what it scales with

`shading_slots()` is `0..next_slot` and `sync_from_resources` registers
every `Material` the `AssetDatabase` holds — **not the scene's, not the
frame's visible ones**. So the bill is per material *in the project*:

| materials in the project | sweeps | cost | of today's 9.7 ms GPU |
|---|---|---|---|
| `roll-a-ball`, 3 | 4 | **0.71 ms** | 7 % |
| 10 | 11 | 1.96 ms | 20 % |
| 20 | 21 | **3.74 ms** | **39 %** |
| 50 | 51 | 9.08 ms | a whole frame |

Of the 3.272 ms shading pass, **up to 0.71 ms is swept without writing a
pixel** — 22 % of it — leaving ~2.56 ms of actual shading. (Up to,
because a real material's sweep does useful work on the pixels it owns;
a padded one never does.)

🔴 **The finding is what it scales with.** A `.ron` dropped into the
project's materials folder and never referenced adds 178 µs to every
frame, silently. At three materials that is tolerable; at twenty it eats
the budget that was just met. Compacting the dispatch is a change to
`ComputeShading::shade` and `MaterialTwoPass::shade` alone.

### The mechanism, which was right even though the number was not

**The shading sweeps the whole screen once per material in the PROJECT.**
A thread in a tile that owns none of this material's pixels still pays a
`textureLoad` of the R64 vbuf, then `visible_meshlets[slot]`, then
`instances[inst_id].material_id` — two dependent storage reads — plus
three unconditional barriers. That description was correct; what was
wrong was believing it added up to 11 ms. It adds up to **178 µs a
sweep**, measured above, and the desktop decomposition says most of that
is the dispatch rather than the threads.

**`motion_vectors` reconstructs the triangle the shading already
reconstructed.** Same pixel, same `textureLoad(vbuf64)`, same payload
decode, same three `global_vertex_id`, same `compute_partial_derivatives`
— and its only own work is `previous_transforms[inst_id]` and one matrix
multiply. The written justification is correct **at half rate**: a
temporal resolve needs a vector per pixel and the shading runs per quad.
At `shading_rate: 1` the two passes do identical work. Fusing them costs
`Rg16Float` -> `Rgba16Float`, 8 B/px against 4, because `Rg16Float` is
not a storage format — the same wgpu format tax FSR 3.1 paid.

❌ **The 2.6 ms this was worth when it was written is now 0.553 ms**, and
the format tax to fuse it is 4 B/px on a bandwidth-bound part. It is no
longer obviously worth doing, and it is not worth doing before somebody
re-measures it.

### The rest of the inventory

**TAA takes nine depth taps a pixel where `textureGather` takes three.**
`taa.wgsl:305-312` is a 3x3 loop of `textureSample(depth,
nearest_sampler, ...)` with a **nearest** sampler, the textbook case for
the instruction — which the repo already uses in `hi_z_spd.wgsl:335` and
`sgsr2_convert.wgsl:104`, so this is asymmetric knowledge rather than a
technique nobody here has.

⚠️ **Two corrections to how this was first written.** The "ten reads"
counted the `(0,0)` tap re-reading `center_depth` from `:284`, and those
two are the same texture, sampler and coordinate — a compiler may fold
them, which was asserted rather than checked. And the **3.7 ms** is the
`taa` scope of a build configured with `upscale: 1`; the preset that
meets the budget uses SGSR 2, where `taa` does not run at all. Whoever
picks this up measures the pass first.

**Resource lifetime, found in passing.** `material_depth_texture`
(`Depth16Unorm`, full screen) is created unconditionally in `new()` and
`resize()` but only the fragment path reads it — ~1.84 MB **per view**,
and the editor runs N. `material_bind_group()` builds a fresh bind group
on every call, one per slot per frame per view.

### Two claims the device refutes, now fixed

`compute_shade`'s header said *"fifteen storage fetches per pixel become
fifteen per tile"*. It cached the **indices**, four bytes each; every
thread still fetches the whole 80-byte `IntiLight`, which is what the
6.6 % measurement above already said and what #826 is for. The dispatch
comment said an idle tile *"costs one vbuf read per thread and then
leaves"* — it is three reads, two of them dependent, plus the barriers.

Both corrected in #909, along with the test that pins
`DOWNSAMPLE_WORKGROUP_SIZE` to the shader. That constant lives in three
places — the host, the shader's `const`, and `@workgroup_size` — and none
of them fails to compile when they disagree; the grid-stride loop simply
steps by a different count than there are threads and the cascade reads
some voxels twice. Its twin `POPULATE_WORKGROUP_SIZE` had the test since
it was written.

✅ Ruled out as false positives, so they are not re-audited:
`SHADING_TILE_SIZE` ↔ `TILE_SIZE` (tested), `enable f16` (injected, tested),
and `compute_shading: false` as the serde default — deliberate, and
`settings.rs:510` argues it well: a default is what an old file silently
becomes, not a recommendation.

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
| **Roll a Ball, phase 0** | **Done.** A ball rolls under WASD and a stick, camera-relative; a raycast gates the jump; a virtual camera follows. Verdict in `lobinuxsoft/roll-a-ball#4`; the plan lives there now, #669 is the pointer |
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

### The audit that changed what "next" means — Roll a Ball

**The plan moved out of this repository on 2026-08-17.** Roll a Ball is a first-user pass over
the whole engine — the engine tested by a project, against the public API only, rather than by
its authors. It has its own repository, its own roadmap and one issue per phase:
**`lobinuxsoft/roll-a-ball`**, tracker issue #3, `docs/ROADMAP.md`. That repository is
**private**; #669 here is now a pointer and the place where its engine-facing findings land.

What stays this document's business is what the exercise *produced* for the engine.

**The audit that started it (2026-07-29) read the tree and found the presentation layer
missing.** Two of its four verdicts have since been paid off:

| Subsystem | Then | Now |
|---|---|---|
| Physics, gravity, meshlet path, materials | strong | strong |
| Input, scripting | present, unproven from a project | proven — and #711 found the backend was connected to nothing |
| **Lights** | **authorable and inert** — nine-line `kooch_lighting`, nothing read the components | **#441 shipped**, with CSM (#476) and contact shadows (#735) behind it |
| Audio | kira backend, no `AudioSource` to author (#63) | unchanged |
| Post (#254), particles (#97), runtime UI (#280/#96) | missing | unchanged |

The lights were the exact shape of the gotcha in `MEMORY.md` — *a missing feature does not fail
the build: the component is authored, mirrored, draws a gizmo, and does nothing* — and a user
placing a light and seeing nothing was, as predicted, the first bug the project found.

**#668 — how systems get to run in parallel**, given that users write their own. Still blocked
on a scene that needs it: a hosting project does **0.17 ms** of work per frame, so there is
nothing to parallelise. Roll a Ball's terrain phase produces that scene.

### What phase 0 found, and why the exercise earns its cost

A ball that rolls, a jump gated on a downward ray, and a virtual camera following it — built in
a project, against the public API only. **#671 phase 1 was proven for the first time**: the rig
had been written since 31 July and had never been seen to move a camera.

The finding that matters is not any single bug. It is that **neither Play showed a working
game, and each failed differently**: remote Play could not receive a key (#710, now closed by
#713), and the direct game lost the camera's target on load (#712, still open). As the engine's
authors, both halves had green tests. Only using it from outside put them in the same room.

Seven separate cases turned up of **complete code with no reachable call site** — the input
crate, `feed_window_event`, the standalone Play path (#720), the dynamic type registry the
prefab inspector never asked, and three in the IDE launcher. **None of them fails a build.**
That is the argument for the exercise, and it is no longer a hypothesis.

**#712 is what the next phase runs into.** Collectibles are prefab work by definition, and a
reference into a prefab instance is dropped on load, at DEBUG, without failing. Phase 0 dodged
it by inlining the ball into the scene; phase 1 cannot.

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
4. **#955 — the editor invents its own scenes.** `SceneManager::new()` seeds one empty
   scene with a random `Guid` and no path, and in remote mode that is the *only* scene the
   editor knows: the project holds a different manager, with the real file, under a
   different id. Nothing carries the open set across the wire.

   🔴 This is the same wall #613 hits from the other side. #613 reads as "additive is
   disabled while a project is open"; the cause is that **the editor has no idea which
   scenes the project has open**, so there is nothing for an additive one to be added *to*.
   Fixing the open set fixes both, and it is what the World panel's tree uncovered — an
   `Untitled (0 entities)` root beside 185 entities claiming to belong to nowhere.

   Blocks seven ordinary panel features that all assume the model is true: unloading on
   non-additive load, selecting a scene row, creating an entity *into a named scene*,
   creating a child in its parent's scene, unparenting by dropping between siblings,
   dropping outside a scene to make a new one, and the rule that **no entity is ever
   loose**.

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
