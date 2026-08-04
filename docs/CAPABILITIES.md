# Capabilities — what exists, and whether anything reaches it

`MEMORY.md` records **why** things were decided. `ROADMAP.md` records **what
comes next**. This file records **what is already built and whether it is
plugged in** — which turned out to be a different question, and the one
nobody was asking.

## Why this file exists

Over two days of using the engine to build a game, the same failure
appeared eight times: **a capability was finished, tested, documented,
merged — and nothing called it.** None of them broke the build. Every one
was found by using the editor, never by reading the code.

- `kooch_input` compiled with zero call sites (#711).
- `feed_window_event` had a doc comment naming a caller that did not exist.
- Play-standalone launches the game in its own process — unreachable from
  the UI (#720).
- `DynamicTypeRegistry` promised in its own docs that the Inspector reads
  it; the prefab inspector never did (#722).
- "Open in IDE" was handed the wrong root by all three call sites (#724).
- The World panel worked out how to draw a full-width list row and wrote
  down why; the asset browser kept using `ui.selectable_label`.
- `Query` — an entire archetype-matching query system — is used by tests
  and one file.
- `RenderGraph` — 497 lines — is instantiated by nobody.

The engine **grows faster than it connects**. This file is the counter to
that: before building something, look here for the thing that already
does it.

## Status vocabulary

| Status | Meaning |
|---|---|
| **connected** | Used in anger by the engine, the editor or a game |
| **internal** | Used only inside its own crate — the `pub use` is noise, not a promise |
| **invisible** | Works, is exported, but the facade prelude does not offer it, so nobody finds it |
| **orphan** | Complete and called by nothing |

`invisible` is the expensive one. Nobody reports it as a bug, because from
outside it is indistinguishable from *not existing*.

## The prelude is the discovery surface

`kooch::prelude` is what a project sees. If a capability is not in it, the
only way to find it is to already know it exists and write the full path.
**That is the difference between `internal` and a feature nobody uses.**

Every entry below marked `invisible` is a prelude line away from being
usable.

## ECS — `kooch_ecs`

| Capability | Where | Status | Notes |
|---|---|---|---|
| `Query<(&A, &mut B), With<C>>` | `query/` | connected (#726) | Archetype matching, `With`/`Without`, `AccessTracker` for conflicting borrows. Used by tests and `scene/propagate.rs`; `kooch_camera`, `kooch_physics` and `kooch_gravity` hand-join storages 37 times instead. The example in its own doc comment is the movement system a game writes on day one. |
| `Commands` | `commands/` | connected | Deferred spawn / insert / despawn. |
| `Transform`, `Name`, `MeshRenderer`, `PerspectiveCamera` | crate root | connected (#726) | The components every scene has. |
| `Component`, `ComponentRegistry`, `Reflect` | `component/`, `reflect/` | connected (#726) | Needed to declare a component at all. |
| `Children` / `Parent` / `GlobalTransform` | `hierarchy/` | connected (#726) | |
| `SceneManager` | `scene_manager/` | connected (#726) | |
| `EntityGuid`, `PersistentIdAllocator` | `persistent_id.rs` | internal | |

**What `Query` does not solve.** A system is `fn(&mut Resources)` — one
handle to everything. Borrowing the registry rules out borrowing the
physics solver, so systems still copy what they need into a local `Vec`,
release, and apply. That scaffolding (`struct Planned` in roll-a-ball) is
not a design pattern; it is the absence of `SystemParam`. Bevy declares
each system's needs in its signature and the scheduler proves the accesses
are disjoint. Kóoch has the query half and not the scheduler half.

## Rendering — `kooch_render`

| Capability | Where | Status | Notes |
|---|---|---|---|
| `RenderGraph`, `RenderNode`, `FnNode`, `NodeId` | `graph/` (497 lines) | **orphan** | DAG + cycle detection + topological sort (Kahn) + shared-encoder execution. PR-1 of #392. Its own module doc lists the follow-ups: *"migration of `SkyRenderPass` and the meshlet stage to graph nodes (separate PRs)"*. Those never happened, and the real renderer was built beside it. **Decide: migrate or delete.** Keeping an unused scheduler that looks authoritative is worse than either. |
| Meshlet pipeline, Hi-Z, deferred, visibility buffer | `meshlet/`, `hi_z/` | connected | The renderer that actually runs. |
| `MaterialPool`, `ImageLoader` | | internal | |
| Frame metrics (`KOOCH_FRAME_METRICS`) | | connected | Env var, silent by default. |

## Input — `kooch_input`

| Capability | Where | Status | Notes |
|---|---|---|---|
| `InputBackend`, `KeyCode`, gamepad ids | `backend.rs`, `ids.rs` | connected | Wired in #711/#713. Own serializable ids for 194 keys / 19 buttons / 8 axes. |
| `ActionMap<A>`, `InputBinding` | ~~`action_map.rs`~~ | **deleted** | Was generic over *your* enum, so the editor could not construct one and a binding could not be serialised at all — authoring in a panel was impossible by construction. Replaced and removed; #55 closed. |
| `.inputaction` assets | `actions/single.rs` | connected | One `Action` per file, composites/processors included. Registers itself at link time. roll-a-ball reads two. |
| `InputAction` component | `actions/single.rs` | connected | Points at an asset by guid, `enabled` per action. Read by `read_input_actions`. |
| `LoadedActions` | `actions/single.rs` | connected | guid → action, reloaded when the file changes. What a game's own component reads through, since a component appears once per entity. |
| Input Map panel | `kooch_editor_core/panels/input_map.rs` | connected | Creates and edits a `.inputaction`: bindings, five composites, processors, modes. |
| Interactive rebind (`BeginRebind`) | `panels/input_map.rs` | **orphan** | The actions exist and nothing emits them: there is no "press any key". The control picker is the only way to bind. |
| `ActionMap`, `priority` | `actions/action.rs` | **orphan** | Survives only as the shape the panel edits — a `.inputaction` opens as a map of one. `priority` is written and never read: stacking maps that consume what they handle was never built, and with per-action `enabled` the remaining gap is bulk enable/disable for a pause menu. |
| `MockInputBackend` | `mock_backend.rs` | **orphan** to games | Injects keys and axes with no hardware — exactly what a cutscene, a tutorial or an automated test needs, and it is reachable only from the engine's own tests. |
| Remote input over the wire | `remote_backend.rs` | connected | `Method::Extension("input.state")`; state, never events. |

## Camera — `kooch_camera`

| Capability | Where | Status | Notes |
|---|---|---|---|
| `VirtualCamera`, `CameraBlend` | `plugin.rs` | connected | |
| `CameraTarget` (tag + group) | `target.rs` | connected | Used by roll-a-ball, which lives in its own repo — measure "unused" against games too, not just this workspace. |

## Editor — `kooch_editor_core`

| Capability | Where | Status | Notes |
|---|---|---|---|
| `SelectableRow`, `row_height` | `widgets.rs` | connected | Full-width list rows. Extracted after the asset browser spent months not having them. |
| Script codegen (module tree) | `actions/codegen/` | connected | Mirrors `src/` folders as a module tree. |
| Play standalone (`handle_play`) | `play_state.rs` | **orphan** | Launches `cargo run -- --game` in its own process, saves the scene to a temp file, captures stdout into the Console — and only runs when *not* remote, while Open Project is always remote. #720. |
| Register Scripts | `actions/asset_ops.rs` | connected but misplaced | Rescans the whole project, yet the button only exists in the context menu of a `.rs` file — so with no `.rs` left there is no way to regenerate. |

## What is still disconnected

Three, and they are the debt this file exists to stop growing. Everything
else in the tables above reached something.

| | Cost of leaving it | Where it goes |
|---|---|---|
| **`ActionMap`** | Input cannot be authored at all — every game hardcodes `KeyCode::KeyW`, and the editor has no panel because there is no data to show it. This is also the open half of the phase-0 verdict in #669. | #55 rewrites the action as data, #58 is the panel |
| **`RenderGraph`** | 497 lines that *look* like the official way to add a pass, next to a renderer that does not use them. The next person to add a pass has to work out which one is real. | migrate the meshlet stage onto it, or delete it — #392 |
| **Play standalone** | The only honest place to tune feel: remote Play costs a frame of latency. Reachable today only by leaving the editor and running `cargo run -- --game` with the env set by hand. | #720 |

## How to keep this honest

- **Adding a capability?** Add its row, and say what reaches it. If the
  answer is "nothing yet", it is `orphan` — write that down rather than
  leaving it implied.
- **`orphan` is a debt, not a state.** Each one carries either a plan to
  connect it or a decision to delete it.
- Counting references outside a crate does **not** find orphans: it flags
  everything that is internal-but-exported, and misses everything a game
  uses from another repo. `CameraBlend` and `HiZ` both failed that test
  and are perfectly alive. Verify each candidate by hand.
- The real detector is **using the engine to build something**. All eight
  cases above were found that way, and none by reading code.
