# Profiling

Where a frame actually goes (#785).

## It is not ours

The flamegraph, the timeline, the frame history, the scope statistics and
the file format are all [puffin](https://github.com/EmbarkStudios/puffin),
drawn by `puffin_egui` into the editor's own egui. GPU-side timing is
[wgpu-profiler](https://github.com/Wumpf/wgpu-profiler). The project's
dependency policy is to never implement what a maintained crate already
covers, and profilers are a solved problem.

What is ours is the panel's capture controls, the scopes, and the
decision about what a build may contain.

## The scope macro is `profiling::scope!`, never `puffin::` directly

[`profiling`](https://github.com/aclysma/profiling) is a facade: one macro
at the call site, and the backend is a cargo feature (puffin, tracy,
optick, superluminal). `wgpu-profiler` is built on the same facade.

⚠️ **With no backend feature enabled, `profiling::scope!` expands to
nothing.** That is what lets a shipped game contain no instrumentation at
all rather than instrumentation that is switched off — see #558 on what a
release build may carry.

| Build | Profiler | Channel |
|---|---|---|
| Editor, `--features profiling` | compiled in | in-panel |
| Game, `--features profiling` | compiled in | `puffin_http` over TCP |
| Release | **absent at compile time** | none |

A happy accident of the facade: **`wgpu` and `egui` already use
`profiling`**, so their internal scopes land in the same flamegraph
without a line of work.

## Recording is off until you ask

Opening the panel records nothing. Puffin's own default for
`are_scopes_on` is `false` and it is left that way; the **Record** button
is the only thing that turns it on.

🔴 There are two transport-looking controls in the panel and they do
different things:

| Control | What it does | Cost while it is like that |
|---|---|---|
| `⏺ Record` / `⏹ Stop` | starts and stops **recording** | stopped: one atomic load per scope |
| `▶` / `⏸` (puffin's own) | freezes the **view** on one frame | recording continues underneath |

**The flamegraph is not drawn while recording.** Drawing it was measured
at 10.97 ms of a 15.98 ms frame — the instrument was two thirds of the
measurement. Record, do the thing, stop, then read.

## Captures

`Save capture` writes a `.puffin` to `~/.local/share/kooch/captures/` —
outside the project, because a capture is a measurement of a session on a
machine and not an asset of the game. `Load capture` reads the newest one
back into the panel, so a capture is readable without installing the
standalone `puffin_viewer`.

`cargo run -p kooch_editor_core --features profiling --example read_capture -- <file>`
prints the same ranking in a terminal, which is what a capture pulled off
the handheld will need.

### 🔴 A capture can be silently unreadable

A `FrameView` turns the scope ids inside a frame into names through a
collection it builds **as frames arrive**. Anything that replaces the view
— `Clear history`, `Load capture` — starts that collection empty, and the
frames that follow come back as `scope#ScopeId(67)`. The capture does not
look broken. It is simply useless.

`GlobalProfiler::emit_scope_snapshot()` asks for every known scope to be
re-sent, and it is called for the first two seconds of every recording.
Not once, and not for two frames, because:

```rust
let propagate_full_delta = std::mem::take(&mut self.propagate_all_scope_details);
...
Err(Error::Empty) => return,   // the frame had no scopes
```

puffin **takes** the flag before building the frame and returns early if
that frame came out empty — carrying the request away. The frame that
closes right after recording starts is exactly that empty frame. And a
scope only registers the first time it runs, so anything occasional
registers after a single snapshot has already gone out.

## Reading the numbers

- **Self time** excludes children. A parent can last 5 ms with 0.1 ms of
  self time; sort by self time for "what costs", read the flamegraph for
  "who is responsible".
- The table's headers sort. It opens sorted by call count, which is the
  least useful column.
- **`Surface::get_current_texture` being the largest entry is not a
  problem** — it is the wait for the compositor. The first release
  capture had it at 2.7 ms of a 4.9 ms frame, with the whole engine
  rendering a viewport in 0.69 ms.
- ⚠️ **`frame` appears twice per editor frame**: the View and Game panels
  each render the scene, with their own cull and shadow passes.

## Profiling the game, which is the point of all of this

Everything above measures the editor: this machine, plugged in, drawing a
viewport. The number the graphics roadmap is judged against is a frame of
a **game on the OneXFly at 10 W**, and no measurement taken here produces
it.

```sh
cargo build --release --features profiling
```

The binary opens `0.0.0.0:8585` and streams every frame to whoever
connects. Nothing else to write: `DefaultPlugins` carries
`ProfilingPlugin` whenever the feature is on, so a game becomes
profilable without its `main.rs` changing.

Then, in the editor's **Profiler** panel, switch the source from *This
editor* to **A running game**, type the handheld's address and press
Connect. The address is remembered in `editor_config.ron`, because it is
a home-network address that is needed every session and wrong in a way
that looks like the profiler being broken.

`puffin_viewer --url 192.168.0.36:8585` reads the same socket, if a
second application is preferable to a panel.

- 🔴 **`0.0.0.0`, not `127.0.0.1`.** Bound to loopback the game is
  reachable only from the handheld, which is the one machine that will
  not be running the viewer. The symptom is a connection that times out
  with nothing logged on either side.
- `KOOCH_PROFILER_ADDR=0.0.0.0:9000` moves it without a recompile, for
  when a build left running on the device is still holding the port.
- Recording is **on from the first frame** here, unlike the editor panel.
  Nobody is going to press Record on a handheld over SSH.
- A port that will not open logs an error and the game keeps running.
  Killing the process someone wanted to measure is the worse answer.
- 🟢 The scope-name problem above does **not** apply to a *late viewer*:
  the server keeps its own `ScopeCollection` and re-sends all of it to
  every client that connects, so one attached an hour in still gets
  names.
- 🔴 It does apply to a **late server**. `scope_delta` is a delta:
  `new_frame` fills it from `new_scopes` and drains that list, so a
  server created after a scope first ran never learns its name and the
  viewer draws `scope#ScopeId(67)` forever. `ProfilingPlugin` runs
  before the first frame, and asks for a snapshot anyway so the
  guarantee does not depend on where it sits in the plugin list.

### One frame boundary, and where it lives

Puffin builds a frame out of the scopes that closed between two
`new_frame` calls. Two boundaries in a frame produce a flamegraph of
half-frames; none produces a single frame that grows forever and never
renders.

The boundary is a system in `Stage::Last`, and it is a *stage* rather
than a line in the loop because there are **two** loops:
`kooch_core::runner::default_runner` for a headless app and
`kooch_window`'s winit loop for a windowed one. A stage runs under both.

⚠️ The editor marks its own boundary, in
`kooch_editor_core/src/systems/render/ui.rs`. The editor does not add
`ProfilingPlugin`; if it ever does, that call goes away in the same
commit or the flamegraph becomes half-frames.

### Stages are named per stage on purpose

`Schedule::run_pre_physics` and friends expand `run_staged!` once per
stage instead of looping over an array. Puffin caches a scope's id in a
`static` belonging to the **call site** and registers it under the first
name that site ever sees — one scope inside `run_stage` would file every
stage of every frame under `Startup`.

### Two things the remote view does backwards, on purpose

- **The flamegraph is drawn while frames arrive.** The local view hides
  it while recording because drawing it cost 10.97 ms of a 15.98 ms
  frame. That cost lands on the machine drawing it, and the frames being
  measured are produced on the other one — the observer is finally
  outside the experiment.
- **Clear reconnects** instead of emptying the view. The collection that
  turns scope ids back into names belongs to the process that recorded
  them, which is on the handheld; there is no `emit_scope_snapshot` to
  call from this side. Reconnecting resets the view *and* makes the
  server re-send every name.

## GPU scopes — the half a CPU profiler cannot see

On the OneXFly the frame is **GPU-bound at 96 %** and the engine's CPU
work is ~2 ms of it. Every CPU scope in this document can say the frame
is slow; none of them can say which pass spends it. `GpuScopes`
(`kooch_core/src/gpu/profiler/`) wraps `wgpu-profiler` to answer that,
and reports the results into puffin so they appear as a **`GPU` thread**
beside the CPU rows rather than in a second tool.

The passes it names, in the order they are recorded:

| Scope | Encoder | What it covers |
|---|---|---|
| `shadows` | meshlet stage | four cascade culls + rasters, plus a cube face per point light |
| `cull` | meshlet stage | the scene-wide meshlet cull dispatch |
| `raster + shade` | meshlet stage | the fused R64 pass — raster *and* the whole lighting evaluation |
| `sky` | game encoder | the raymarch #771 accuses |
| `blit` | game encoder | the stage's colour composited over the sky |

Turned on by the same `--features profiling` as everything else. A build
without it carries a `GpuScopes` whose every method compiles to nothing,
so the render code has one shape rather than a `cfg` at each pass.

### The API is `begin` / `end`, and both halves live on one encoder

`wgpu_profiler::Scope` borrows the encoder for the scope's lifetime,
which leaves the code being measured with no encoder to record into.
`begin` returns a query and hands the encoder straight back.

🔴 **A scope must close on the encoder that opened it.** It pushes a
debug group, and wgpu rejects the encoder outright at `finish()` —
*"A debug group was not popped before the encoder was finished"*. A
profiling build would panic where a release build runs.

🔴 **Nesting is by declared parent, not by call order.** A scope opened
while another is open is *not* its child; `begin_child` is. Left to
`begin`, a pass and the pass containing it come back as siblings and
their times read as additive.

### The GPU clock is not puffin's clock

A GPU timestamp's absolute value is undefined — `wgpu-profiler` says so
in as many words. Reported raw, the GPU track lands an arbitrary distance
from the CPU track and the viewer draws a frame stretched across the gap
with both ends too small to read. `puffin_bridge` translates each batch
so it **ends at now**.

⚠️ **Durations and nesting are exact; the position on the axis is not.**
The results belong to a frame a few submits back, and wgpu exposes no
calibrated timestamp to correlate the two clocks with. Read a GPU row for
how long a pass took, never for what a CPU row was doing at that instant.

### 🔴 `wgpu-profiler`'s own puffin feature cannot be used here

It depends on puffin ^0.19.1, and this workspace patches puffin to 0.20.
Enabling it yields either an unresolvable lock or the two-`GlobalProfiler`
failure described at the bottom of this page. `puffin_bridge.rs` is its
`src/puffin.rs` adapted — 45 lines against API that is identical between
the two versions — and `wgpu-profiler` is taken with
`default-features = false`.

⚠️ `TIMESTAMP_QUERY` is **three** separate wgpu features. Scopes on an
encoder need `TIMESTAMP_QUERY_INSIDE_ENCODERS` specifically;
`gpu/features.rs` requests all three, conditionally, and an adapter
missing them yields scopes that measure nothing instead of a failed
submit.

## What is not built yet

- **GPU scopes in the editor.** The editor builds its own render stage in
  `kooch_editor_core/src/systems/startup.rs` rather than through
  `RenderPlugin`, so no `GpuScopes` reaches its `Resources` and no scope
  it would open is ever resolved. The game is where the 96 % was
  measured; the editor is the follow-up.
- Scopes finer than a pass. `raster + shade` is one box on both axes, and
  it is the box the per-pixel cost is expected to be inside.

## ⚠️ Four dependencies come from git, temporarily

`puffin`, `puffin_egui`, `puffin_http` and `profiling` are patched to git
revisions. Their released versions are one migration behind: `puffin_egui`
0.30.0 pins egui 0.33 against this workspace's 0.35, and `profiling`
1.0.17 pins puffin 0.19 against the panel's 0.20.

Two egui versions make the panel's `Ui` a different type from ours. Two
puffins are worse: **two separate `GlobalProfiler`s**, every scope
recording into one and the panel reading the other, and the symptom is an
empty flamegraph with nothing to blame.

They are `[patch.crates-io]` rather than plain git dependencies so the
whole graph resolves to one copy, and pinned by `rev` rather than branch.
**Remove the section when the releases land.**
