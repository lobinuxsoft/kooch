# Roadmap

What is next and why, ordered by what blocks what. Issue bodies hold the detail; this is the
map.

Companion to [`MEMORY.md`](MEMORY.md), which records decisions already made. If the two
disagree, `MEMORY.md` wins on *decisions* and this file wins on *order*.

Last updated 2026-08-02, `development` at `a85b777`.

---

## Done recently

| | |
|---|---|
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

### Next — the action becomes data, and the player becomes parts

**#55 — input actions as data.** `ActionMap<A>` is generic over a Rust *type*, which cannot be
serialised, inspected or edited — so the editor can never author a binding. And
`InputBinding::GamepadButton(GamepadId, …)` stores a runtime device id, so a gamepad binding
cannot be written without the gamepad plugged in. This is the model **#58**'s binding panel
draws, so it goes first.

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

## Next — editor, because multi-scene is half-reachable

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
