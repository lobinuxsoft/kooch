# Profiling

Where a frame actually goes (#785).

## It is not ours

The flamegraph, the timeline, the frame history, the scope statistics and
the file format are all [puffin](https://github.com/EmbarkStudios/puffin),
drawn by `puffin_egui` into the editor's own egui. GPU-side timing will be
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
| Profiling build preset | compiled in | `puffin_http` over TCP *(not built yet)* |
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

## What is not built yet

- **Connecting to a game running on the target hardware** — the point of
  the exercise. `puffin_http` in the game, the editor as its client, and a
  build preset that produces the instrumented binary.
- **GPU scopes** via `wgpu-profiler`. ⚠️ `TIMESTAMP_QUERY` is three
  separate wgpu features and asking for the wrong one fails at submit.
- Scopes finer than the pass level. `frame` is currently one box.

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
