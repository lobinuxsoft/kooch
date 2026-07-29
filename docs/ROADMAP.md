# Roadmap

What is next and why, ordered by what blocks what. Issue bodies hold the detail; this is the
map.

Companion to [`MEMORY.md`](MEMORY.md), which records decisions already made. If the two
disagree, `MEMORY.md` wins on *decisions* and this file wins on *order*.

Last updated 2026-07-29, `development` at `a2bf355`.

---

## Done recently

| | |
|---|---|
| **#605** | The custom ECS stays. `bevy_ecs` evaluated on measurements and declined |
| **#607** | Stable entity references — a component can point at an entity and survive a save |
| **#609** | More than one scene loaded at once |
| **#612** | Parented physics — compound colliders, gizmo parent-space conversion, Inspector warnings |
| **#560** | Joints — one `Joint` component, all eight rapier kinds, motors, limits, breaking |
| **#618** | Mass control — `mass` means kilograms, shapes are massless, explicit centre of mass |
| **#563** | Physics debug render — the solver's own account of itself, in the viewport |
| **#623** | Collider material — friction, restitution, combine rules and damping, all authorable |
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
| **#656** | The editor and its project stopped burning two cores to show a still image. 200% → 3% CPU, 51.8 → 31.5 W, measured on the same host |

---

## Next — performance and the files that hide it

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

1. **#645 — the remote pull blocks the editor's main thread every frame during Play.**
   `REFRESH_INTERVAL_PLAYING = 1` in `systems/remote_sync.rs` → `session.refresh()` →
   `client.list_entities()`, inline in the frame. The editor is not waiting on the transport;
   it is waiting for the project process to reach its next `Stage::First`. Still unmeasured —
   the transport changed under it (#654), the instrumentation did not get written. Candidates:
   move it off-thread, diff server-side, or drop to a cadence with interpolation.
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
3. **#666 — the editor gathers a full snapshot of the world every frame, for every panel,
   visible or not.** `frame_display.rs::gather` walks entities, archetypes, component types,
   reflected types and scenes before anyone decides whether to draw. With #656 done, this is
   what is left on an idle frame, and it scales with the scene rather than with the UI.
4. **`asset_browser/tree.rs::render_root` rebuilds the whole folder tree every frame, twice**
   (Project and Engine roots), cloning a `PathBuf` per node. ~12 assets today, so invisible;
   the same shape as the Console bug that was not.
5. **Panels with unbounded lists are not virtualised.** The Console now is (#643). The
   hierarchy in `panels/world.rs` is not, and a large scene is exactly when it matters.
6. **`ome_gravity::plugin` walks and allocates its source list twice per frame** — once in
   `reconcile_world_gravity`, once in `apply_gravity_sources`. Small, but it is per frame.
7. **#569 — per-stage counters in the perf HUD.** Out of order on purpose: without it every
   item above is argued rather than measured. Consider doing this *first*.

**The rule this session earned:** egui redraws everything every frame, so whatever a panel
does in its `draw` it does sixty times a second for as long as it is visible. The user's
report was *"it depends on how many panels are open"*, and that was exactly right.

### 2. The monolithic files — done, with one exception

Thirty files were over 400 lines. PR #653 split them. What is left over 600 is a single file,
and it was skipped on purpose:

| Lines | File | Why it is still there |
|---|---|---|
| 1148 | `ome_editor_core/src/actions/remote_edit.rs` | Skipped as demolition-bound. **That premise is dead** — the remote stays (#647), so this file has to be split like the rest |

Twenty-five files sit between 400 and 600. That band is no longer an automatic split: the
threshold moved to 600, above which a file is *examined* for whether it went monolithic or is
carrying something that does not belong to it. Size alone is not the verdict. See `MEMORY.md`,
"Cómo se parte un archivo monolítico".

The full list, any time: `find crates src examples -name '*.rs' | xargs wc -l | sort -rn |
awk '$1 > 600'`.

### Then, the features that were next before this

**#562** (scene queries), **#567** (PD/PID controllers), **#639** (split `RigidBody` into
`RigidBody`/`KinematicBody`/`StaticBody` — a scene-format migration).

> The standing rule: implement what Rapier offers, warn for what it does not. See `MEMORY.md`.

---

## Next — editor, because multi-scene is half-reachable

1. **#619 — no way to create a scene.** There is no New Scene. Multi-scene cannot be
   exercised without files already on disk.
2. **#613 — additive scenes over the wire.** The protocol assumes one scene, so additive
   loading is disabled while a project is open. The only place a merged feature is
   knowingly incomplete.
3. **#591** context menus, **#592** a Game panel separate from View.

---

## Then — prefabs

**#611**, in two phases. Phase A is runtime instancing: a scene instanced with its entity
ids remapped. Phase B is the linked-with-overrides prefab system.

Two things phase A must settle because they touch merged types: whether a scene needs a
single root, and how an outside reference names *this instance* rather than the prefab —
`EntityRef::Persistent { scene, id }` is ambiguous once a scene is instanced twice.

---

## Larger, not yet started

- **#566 — world cells.** Scenes become streamable content; entities transit between cells.
  Scene and cell are orthogonal axes. This is the piece planet-scale actually needs.
- **#614 — terrain LOD.** Research, with an honest verdict: dual contouring beats
  marching-cubes-plus-Transvoxel on an octree, but feeding octree nodes through the meshlet
  pipeline is **an unproven hypothesis**. Nobody found doing it. Measure the cost of
  clusterising one dirty node before any design commits.
- **Rendering backlog** — #476/#477 shadows, #450 GI, #485 clustered light culling, #484
  HDR, #481 motion vectors and FSR. Unblocked, unscheduled.
- **#558** — shippable builds must exclude the editor. Security, not size.

---

## Deliberately not scheduled

- **Adopting Bevy wholesale.** Would save roughly two thirds of the codebase and cost the
  parts that are the point: the GPU-driven meshlet renderer, planet-scale streaming, and the
  editor — none of which Bevy provides. Settled in #605.
- **Making a dynamic body follow its parent.** No engine supports it; Godot has failed to
  for years. The supported answers are compound colliders (#615, done) and joints (#560).
