# Roadmap

What is next and why, ordered by what blocks what. Issue bodies hold the detail; this is the
map.

Companion to [`MEMORY.md`](MEMORY.md), which records decisions already made. If the two
disagree, `MEMORY.md` wins on *decisions* and this file wins on *order*.

Last updated 2026-07-27, `development` at `ea72349`.

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

---

## Next — performance and the files that hide it

Two sessions running, the thing that actually went wrong was not a missing feature. It was
work done per frame that did not need doing, in files too big for anyone to notice. Both are
the same problem seen from two sides, so they are one push.

### 1. Per-frame work that should not exist

Ordered by measured or estimated cost. **Measure first, then fix** — the last two wins came
from a number, and the two guesses before them were wrong.

1. **The remote pull blocks the editor's main thread every frame during Play.**
   `REFRESH_INTERVAL_PLAYING = 1` in `systems/remote_sync.rs` → `session.refresh()` →
   `client.list_entities()`, a synchronous HTTP round-trip plus a parse of the whole scene
   snapshot, inline in the frame. This is the largest suspected cost in the editor and has
   never been measured. Candidates: move it off-thread, diff server-side, or drop to a
   cadence with interpolation.
2. **#641 — egui `changed id between passes`.** 1919 warnings in a four-minute session, up
   from 450 in a ten-minute one: it scales with panels open. Each is a `format!` of two
   `Vec<Id>` plus a write to stdout *and* the log buffer. It is also a correctness bug — a
   widget whose id changes between passes loses its interaction state.
3. **`asset_browser/tree.rs::render_root` rebuilds the whole folder tree every frame, twice**
   (Project and Engine roots), cloning a `PathBuf` per node. ~12 assets today, so invisible;
   the same shape as the Console bug that was not.
4. **Panels with unbounded lists are not virtualised.** The Console now is (#643). The
   hierarchy in `panels/world.rs` is not, and a large scene is exactly when it matters.
5. **`ome_gravity::plugin` walks and allocates its source list twice per frame** — once in
   `reconcile_world_gravity`, once in `apply_gravity_sources`. Small, but it is per frame.
6. **#569 — per-stage counters in the perf HUD.** Out of order on purpose: without it every
   item above is argued rather than measured. Consider doing this *first*.

**The rule this session earned:** egui redraws everything every frame, so whatever a panel
does in its `draw` it does sixty times a second for as long as it is visible. The user's
report was *"it depends on how many panels are open"*, and that was exactly right.

### 2. The monolithic files

Thirty files over 400 lines. The Console bug lived in one and was invisible until it was
split. Ordered by size; the ones carrying per-frame work are worth doing first.

| Lines | File |
|---|---|
| 1136 | `ome_editor_core/src/actions/remote_edit.rs` |
| 709 | `examples/physics_smoke.rs` |
| 620 | `ome_physics/src/components/body.rs` |
| 612 | `ome_physics/src/rapier_backend/backend.rs` |
| 595 | `ome_editor_core/src/queries.rs` |
| 595 | `ome_editor_core/src/panels/asset_browser/tree.rs` |
| 562 | `ome_editor_core/src/actions/handlers.rs` |
| 535 | `ome_editor_core/src/gizmos/visibility.rs` |
| 507 | `ome_editor_core/src/panels/inspector/physics_warnings.rs` |
| 499 | `ome_render/src/material/pipeline.rs` |
| 488 | `ome_editor_core/src/actions/codegen.rs` |
| 475 | `ome_editor_core/src/systems/render.rs` |
| 466 | `ome_editor_core/src/gizmos.rs` |

Plus test and example files over the line (`tests/fields.rs`, `meshlet_bench.rs`,
`plugin/tests/joints.rs`, `make_playground.rs`, and others) — lower priority, since a test
file's size costs review attention rather than runtime.

`cargo run --example` and the full list: `find crates src examples -name '*.rs' | xargs wc -l
| sort -rn | awk '$1 > 400'`.

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
