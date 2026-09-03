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
- `unload_project_plugins` had **zero callers**, so the editor could load a
  project's code and never swap it — reopening the project was the only way
  to see a component change (#733).

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
| `surface_reconstruct.wgsl` | `shaders/` | connected (#441) | Barycentric world position / normal / uv / tangent, shared by both shading paths. Was the R64 path's alone; the R32 fallback averaged vertex normals and had no world position, which only stopped being invisible when a point light needed a distance. |
| `MeshletDebugMode::Normals` | `meshlet/debug.rs` | connected (#441) | The old shading model, demoted to a dropdown entry. The discriminant is pinned by a test because two WGSL files compare against a literal `11u`. |
| `MaterialPool`, `ImageLoader` | | internal | |
| Frame metrics (`KOOCH_FRAME_METRICS`) | | connected | Env var, silent by default. |

## Lighting — `kooch_lighting` (Inti)

Until #441 this crate was **nine lines**: a doc comment promising point, spot, directional and
area lights, volumetrics and bloom, plus an `init()` that logged. Nothing in the engine called
it. The three light components existed, the editor drew their gizmos, the Inspector edited
them, the remote protocol mirrored them — and no render crate read one.

| Capability | Where | Status | Notes |
|---|---|---|---|
| `GpuLight` | `gpu_light.rs` | connected (#441) | 64 B `repr(C)` record. Direction from the transform, never a field. Spot cone pre-packed as the MAD the shader evaluates. AoS on purpose — every invocation reads all of one light; SoA is what *culling* will want, and that is a different buffer. |
| `extract_lights` | `extract.rs` | connected (#441) | The ECS walk, pure and GPU-free. Warns past 256 lights and never clips. |
| `GpuLights` | `buffer.rs` | connected (#441) | Buffer residency, geometric growth, one bind group for both shading paths. |
| `AmbientLight`, `Exposure` | `frame.rs` | connected (#744) | Was unreachable from the editor; now authored in a `.rendersettings` asset and applied per frame. `PhysicalCamera` (aperture / shutter / ISO) is the control worth using — EV100 is correct and unusable. |
| `PhysicalCamera` | `frame.rs` | connected (#744) | Presets: `sunny()` EV 15, `default()` EV ≈ 9.9, `indoor()` EV 7. |
| Shadows | `kooch_render::shadow` | connected (#476) | Cascade fit, split scheme, LOD selector, filter and both biases all ported from Bevy 0.19 — see the roadmap for what that cost and why. |
| `ShadowSettings` | `shadow/settings.rs` | connected (#476) | Distance, cascade resolution and an off switch, authored in `.rendersettings`. Reached the pass on the same frame — the Resource-with-no-UI failure has now been committed three times and this is the version that is not it. |
| Alpha-cut shadows | — | **not built** | The depth pass has no fragment stage, so foliage casts the shadow of its quad. Needs a second pipeline for the materials that ask. |
| Punctual shadows | — | **not built** | Only the directional light casts, and only the first one. #734. |
| `inti_pbr_shader(group)` | `lib.rs` | connected (#441) | The shading model as WGSL, bind-group index substituted textually. Concatenated by both paths so the BRDF cannot fork. |
| Volumetrics, bloom, area lights | — | **not built** | The crate's original doc comment promised all three. It now promises what it has. |

## Assets — cross-crate

| Capability | Where | Status | Notes |
|---|---|---|---|
| `register_reflected_asset!` | `kooch_ecs::reflect::asset_registry` | connected (#744) | An asset type registered with it is **editable in the Inspector with no editor changes**. Before it, a new asset type cost three edits in `kooch_editor_core` and anything missed displayed "No import settings for X". |
| Scan adoption | `kooch_core::asset_database::scan` | connected (#744) | A file with no `.meta` is adopted when a registered loader claims its extension. Broke the circle where the browser showed what the database registered, the database registered what had a `.meta`, and the `.meta` appeared when something loaded the file — so a hand-written file was invisible forever. `MEMORY.md` recorded the symptom twice before anyone followed it to the `continue` causing it. |
| `RenderSettings` | `kooch_render::settings` | connected (#744) | The project's `.rendersettings`. Absent, the engine defaults apply and nothing errors. |
| Field tooltips | derive + 3 bridges | connected (#737) | The `Reflect` derive harvests `#[doc]`; it travels in-process, over the plugin ABI and over the remote protocol. The third is the one that mattered — Open Project always opens remote. |

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

## Physics — `kooch_physics`

| Capability | Where | Status | Notes |
|---|---|---|---|
| `PhysicsBackend` trait | `backend/mod.rs` | connected | The contract. Rapier lives only inside `rapier_backend`; every public type is glam, so a GPU solver later touches no authored scene. |
| `CollisionShape` — 15 variants | `backend/shape.rs` | connected | Every shape rapier 0.34 ships. Analytic ones from typed numbers, mesh-derived ones from `ColliderMeshCache`. |
| `ColliderMeshCache` | `backend/mesh_cache.rs` | connected | The one seam pointing outward. Defined in physics, filled by `kooch::collider_meshes` — the facade is the only crate that sees both a GUID and an asset database. |
| Surface + filtering | `components/body/collider/` | connected | `friction`, `restitution` and their combine rules; four group masks; the sensor flag. All per collider, all baked at build time, so an Inspector edit retires and rebuilds the body. |
| Compound bodies | `plugin/compound.rs` | connected | A descendant `Collider` with no body of its own joins the nearest ancestor that has one, at its own local pose. A dynamic body under another warns. |
| Baked collision meshes | `kooch_editor_core/actions/handlers/collider.rs` | connected | "Create hull mesh" / "Create convex parts" in a mesh asset's Inspector. Writes into the open project, records the source GUID + a byte hash, and says so when the source has moved on. |
| Collision mesh loading | `kooch::collider_meshes` | connected | Parses the `.glb` directly. Going through `MeshletMesh` built a LOD chain — 2.9 s on a 76k mesh, measured in debug — and decoded it straight back to triangles. |
| `Heightfield`, `Segment`, `Triangle`, `Polyline`, `Voxels`, `VoxelizedMesh` | `backend/shape.rs` | **invisible, deliberately** | Build, collide and are tested; none is in the dropdown. A height grid cannot be typed and there is no terrain asset to read one from; the other five are answers to questions no author has, and listing them made the menu a quiz. Discriminants kept, so a scene authored with one still loads. |
| Collider gizmo | `kooch_editor_core/gizmos/collider.rs` | connected | Every analytic shape at its effective size, plus hulls and convex pieces outlined from the cache the solver reads. A triangle mesh draws nothing on purpose: it *is* the render mesh. |
| Scene queries | `backend/query.rs`, `rapier_backend/backend/queries.rs` | connected | Shape cast, point projection, overlap test, multi-hit ray, all filtered. A ray is a line of zero width: it slips between two crates a body could not fit through and misses the thin wall a fast projectile would hit. Sweeping the shape that is actually moving is the honest question. |
| `QueryFilter` | `backend/query.rs` | connected | Excludes a body, narrows by group, skips sensors. Filtering in the pipeline rather than the caller: a character's ground probe finds *itself* first, and discarding the nearest hit still misses the second collider on the same body. |
| `Visualizer::draw_with` | `kooch_gizmos/visualizer.rs` | connected | The overload that gets the entity **and** `Resources`, for an outline whose geometry lives outside its component. It had no entity at first, so a gizmo could not read the component beside it — and `CharacterController::ride_height` documented a trap the gizmo was supposed to draw and could not. Defaults to `draw`. |
| `CollisionShape::reach` | `backend/shape.rs` | connected | How far a shape hangs below its own origin. `None` for a point cloud and for the unbounded ones, because drawing a reach of zero would say "any ride height clears this". |
| `PhysicsWorld::set_rotation` | `plugin/world/mod.rs` | connected | Turns a body and leaves it where it is, for an orientation that is authored rather than simulated. Pair it with `set_angular_velocity` or the solver turns the body straight back out of the pose. |

## Gravity — `kooch_gravity`

| Capability | Where | Status | Notes |
|---|---|---|---|
| `GlobalGravity` | `sources/global.rs` | connected | The world vector as a component, so a level can author and switch it. |
| Rigid field space | `plugin/collect.rs` | connected | Every source's distances are metres: a transform places a field and never resizes one. It carried the scale once, so `range: 20` on an entity scaled to 8 pulled from 160 m while the Inspector read 20 — and three sources scaled while `PointGravity` did not. |
| `PointGravity` | `sources/point.rs` | connected | A planet. `strength` at a `radius` rather than `G·M`, clamped inside that radius so the pull towards a centre stays finite. |
| `AreaGravity` | `sources/area.rs` | connected | A box you are *inside*, with its own down. Rotates with its entity. |
| `BoxGravity` | `sources/box_field.rs` | connected | A solid you stand on the *outside* of — the gradient of a rounded box's SDF, so faces, edges and corners come out consistent with no special case. |
| `PlaneGravity` | `sources/plane.rs` | connected | A floor: bounded along its normal, unbounded across it, and one-sided. What an area with enormous half-extents was pretending to be. |
| `GravityPriority` | `sources/priority.rs` | connected | The zone that overrules rather than joins. Suppresses lower levels in proportion to its own reach, so crossing the boundary is a fade and not a snap. |
| `gravity_at` / `gravity_up` | `plugin/mod.rs` | connected | The summed field, and which way is up in it. `gravity_up` is what the camera's `UP_GRAVITY` asks. |
| `gravity_dominant` | `plugin/mod.rs` | **still invisible** | Up according to the strongest single source, for orientation rather than force. This row used to say "nothing calls it until the character controller (#94) does" — #94 landed and reached for `gravity_at` instead, because a character standing where two fields overlap should stand in their *sum*, not in whichever happens to be winning. Still exported, still uncalled. |
| Impulse, not force | `plugin/apply.rs` | connected | Rapier's forces persist until `reset_forces`, so gravity as a force would compound. `mass × acceleration × dt` is equivalent over the step and composes with gameplay. Sleeping bodies are skipped unless the field itself changed, which is the whole reason a settled scene stays cheap. |
| GPU buffer of sources | — | **not built** | No consumer. The solver is rapier and rapier is CPU, so a buffer on the GPU would have to be read back to be used. It arrives with whatever needs to ask which way is down *on* the GPU. |

## Character — `kooch_character`

One pass senses; every mechanic reads it. Two systems each casting their
own ground ray is how `apply_movement` and `apply_jump` came to be wrong
in the *same* way, both reading "grounded" in mid-air — rapier casts rays
solid, so a downward one from a body's own centre finds that body at zero
distance. Casting belongs to the sense pass; deciding belongs to the
mechanics.

| Capability | Where | Status | Notes |
|---|---|---|---|
| `sense::under` | `plugin/sense.rs` | connected | Sweeps a sphere down and says whether it found `Ground`, a `Step` or a `Wall`. A step's riser and a wall give the *same* contact normal, so it drops a second probe just past the contact and looks for a ledge within `step_height`. Refusing by the normal alone turned a climbed 0.6 m step into `rose 0.048`. |
| `sense::beside` | `plugin/sense.rs` | connected | Ahead **and to both sides**, and only for a character that authored a `Touching`. A probe aimed where the character is going never finds the wall it is running *along* — 60 frames out of 60 with no wall. |
| `Grounded` | `grounded.rs` | connected | What is underneath, written once a step. |
| `Touching` | `touching.rs` | connected | The same for walls. Written only where the component is authored, so a character with no use for walls does not pay three sweeps. |
| `CharacterController` | `controller.rs` | connected | The spring: `ride_height` from the **origin**, so it has to clear the collider's own reach. `damping` is the feel dial — at critical the landing dips 0.000 m, and a third of critical dips 0.27 and recovers. |
| `Walk` | `walk.rs` | connected | A goal velocity, not a push. A floating capsule never touches the floor so it has **no friction at all**; asking for a velocity makes stopping the same term as starting. In the air it only ever *adds*, and never into a wall — shoving one bought nothing but the contact friction that glued the character to it at 0.8 m/s² of fall. |
| `Facing` | `facing.rs` | connected | Where gameplay is steering, written **every frame**. Its length is the throttle, so a system that skips writing zero leaves the character walking on its own for ever. The name is wrong for what it does and is worth changing. |
| `Sprint` | `sprint.rs` | connected, **no system** | It applies no force — it scales two numbers `Walk` already has. A mechanic that adds a term gets a system; one that scales an existing term is a modifier. |
| `Jump` / `WallJump` | `jump.rs`, `plugin/leap.rs` | connected | A launch *speed*: `speed² / 2g` is a height a designer can aim at, where an impulse over mass is not. Air jumps, coyote time and an input buffer, because nobody presses the button on the frame they meant to. Coyote time costs the ground jump, or walking off a ledge would be worth more than standing still. |
| `WallSlide` | `wall_slide.rs` | connected | Caps the fall rather than applying a friction, so the slide speed does not depend on how far the character had already fallen. `stick` is the hold that used to come free from contact friction. |
| `WallRun` | `wall_run.rs`, `plugin/run.rs` | connected | The other answer to a wall, and a different move rather than a setting of the slide. Needs speed **along** the wall on arrival — asked once, because asking every frame let a character arrive at walking pace and steer itself up to running speed against the wall. |
| `CharacterVisualizer` etc. | `kooch_editor_core/gizmos/` | connected | The ride height, the probe, the slope cone, the measured contact, both headings, the goal velocity beside the real one, and the wall. Every one of these numbers is invisible until something falls through the floor. |
| Kinematic alternative | — | **not built** | #94 offered it and the floating capsule answered every case that came up. It arrives when something needs a body the solver does not argue with. |

## Camera — `kooch_camera`

| Capability | Where | Status | Notes |
|---|---|---|---|
| `VirtualCamera`, `CameraBlend` | `plugin.rs` | connected | |
| `CameraTarget` (tag + group) | `target.rs` | connected | Used by roll-a-ball, which lives in its own repo — measure "unused" against games too, not just this workspace. |
| `HorizonFrames` | `plugin.rs` | connected | Where each vcam measures yaw from, carried between frames and transported onto each new up. Deriving it from `up` alone is impossible without a pole — the hairy ball theorem — and the pole was a 180° flip at one spot on every planet. |

## Assets — `kooch_core::asset_loader`

| Capability | Where | Status | Notes |
|---|---|---|---|
| `AssetServer::load`, `load_by_guid` | `server.rs` | connected | Path and guid cache, `.meta` identity on first load. |
| `AssetServer::reload_path` | `server.rs` | connected | Overwrites the slot existing handles point at, so a reload is visible to everything already holding one. Type-erased: the caller has a path, which is all a save handler or a wire message ever has. |
| `asset_written` | `written.rs` | connected | The one thing a save calls — registers identity, then refreshes. Used by the editor's `asset_saved` and by the host's `ReloadAsset` handler, so both processes take the same path. |
| `Method::ReloadAsset` | `kooch_remote/handlers.rs` | connected | Any asset, not just prefabs. Was `ReloadPrefab` + `forget::<SceneDocument>`. |
| `AssetServer::forget` | `server.rs` | **orphan** | Drops a cache entry so the *next* load re-reads. Nothing calls it any more: it mints a new key, so everything already holding a handle keeps the old bytes — which is why `reload_path` exists. Kept as the honest primitive under it; delete if it stays unused. |
| Asset tree scan | `systems/project_assets.rs` | connected, partial | Runs on project open/change only. A file created outside the editor mid-session is still invisible until reopen — the editor's own writes are covered by `asset_saved`. |
| File watching | — | **absent, deliberately** | No `notify`, no polling. The editor writes these files, so it already knows; and this repo lives on NTFS through FUSE, where inotify is unreliable and mtime resolution is coarse enough to miss two saves in the same second. |

## Editor — `kooch_editor_core`

| Capability | Where | Status | Notes |
|---|---|---|---|
| `SelectableRow`, `row_height` | `widgets.rs` | connected | Full-width list rows. Extracted after the asset browser spent months not having them. |
| `asset_saved` | `actions/handlers/prefab.rs` | connected | Every write of an asset goes through it: prefab, material edit, material creation, input action, import, duplicate. Was two prefab-only helpers, which is why the other five did nothing. |
| Script codegen (module tree) | `actions/codegen/` | connected | Mirrors `src/` folders as a module tree. |
| Play standalone (`handle_play`) | `play_state.rs` | **orphan** | Launches `cargo run -- --game` in its own process, saves the scene to a temp file, captures stdout into the Console — and only runs when *not* remote, while Open Project is always remote. #720. |
| `reload_project_plugins` | `project_plugin.rs` | connected | Unload + load as one operation, with the type registry restored when the new library declares nothing. Was two halves that had never run in sequence: `unload_project_plugins` had no callers at all. |
| `CodeReload` | `code_reload.rs` | connected | Stats the project's `.so` once a second and swaps it when it moves. Polls rather than watches, for the same reason `script_sync` does: inotify drops events on this FUSE mount. |
| `Reloaded` | `project_plugin/reload.rs` | connected | Names what a swap changed — a lost type, a dropped field, a field that changed kind. The engine breaks data rather than migrating it, which is only safe while breaking is loud. |
| Register Scripts | `actions/asset_ops.rs` | connected but misplaced | Rescans the whole project, yet the button only exists in the context menu of a `.rs` file — so with no `.rs` left there is no way to regenerate. |

## What is still disconnected

The debt this file exists to stop growing. Everything else in the tables
above reached something.

| | Cost of leaving it | Where it goes |
|---|---|---|
| **`RenderGraph`** | 497 lines that *look* like the official way to add a pass, next to a renderer that does not use them. The next person to add a pass has to work out which one is real. | migrate the meshlet stage onto it, or delete it — #392 |
| **Play standalone** | The only honest place to tune feel: remote Play costs a frame of latency. Reachable today only by leaving the editor and running `cargo run -- --game` with the env set by hand. | #720 |
| **Interactive rebind** | The panel can bind through a picker, so this is polish rather than a hole — but `BeginRebind`/`CancelRebind` exist and nothing emits them, which reads as a feature. | emit them, or delete them |
| **`MockInputBackend`** | Injecting input without hardware is what a cutscene, a tutorial and an automated gameplay test all need, and a game cannot reach it. | expose it through the prelude |
| **`ActionMap::priority`** | Written, never read. The one thing lost when the map was deleted is bulk enable/disable, and its consumer is a pause menu that does not exist yet. | when a pause menu lands |

**Resolved since the last pass:** `ActionMap` (the action is an asset now,
#55/#58 closed) and asset staleness — a saved file used to reach only the
editor, or only prefabs.

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
