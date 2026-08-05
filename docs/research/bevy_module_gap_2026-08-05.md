# Bevy 0.19's 53 modules against our 18 crates

Release notes say what is *new*. This says what **exists**. Read from
`docs.rs/bevy/0.19.0` module index, cross-checked against our tree by
grep, not memory.

Judged against the goal: **universes**. A module that only matters in a
room is marked as such.

## Covered — we have an equivalent

| Bevy | Ours | Note |
|---|---|---|
| `app`, `ecs` | `kooch_core` + `kooch_ecs` (16k lines) | Ours is hybrid CPU-GPU; see the ECS comparison in the sweep |
| `asset` | `kooch_core::{asset_loader, assets, asset_database}` | Ours has `.meta`, a database and hot reload |
| `audio` | `kooch_audio` — **697 lines, no `AudioSource` component** | Shell. #63/#64/#65 |
| `camera`, `camera_controller` | `kooch_camera` (2k) + the editor's own | Their controllers went first-party in 0.18; ours is #671 |
| `gizmos`, `gizmos_render` | `kooch_gizmos` + `kooch_gizmos_handles` | Ours is immediate mode; theirs got retained in 0.16 (~65–80×) |
| `gltf` | `kooch_render::gltf_loader` | |
| `input`, `gilrs` | `kooch_input` (4.5k) | Ours has its own serialisable ids for the remote protocol |
| `input_focus` | `kooch_editor_core::input_focus` | Written this session, same name, arrived at separately |
| `light` | `kooch_ecs` components + **`kooch_lighting`, 9 lines** | 🔴 Authorable and inert. #441 |
| `log` | `kooch_core::{log_console, LogBuffer}` | |
| `math` | `glam` directly | They wrap it; we do not, and #657 already noted glam is not re-exported |
| `mesh`, `pbr`, `render`, `core_pipeline`, `material`, `shader`, `image` | `kooch_render` (21k) | Meshlet path is ours and is strong |
| `picking` | `kooch_editor_core::viewport_pick` | Theirs is engine-level and works for gameplay too; ours is editor-only |
| `reflect` | `kooch_ecs` reflect + `DynamicTypeRegistry` | Ours reaches components from a project `.so`. Theirs has no editor to need that |
| `remote` | `kooch_remote` (2.6k) | Same concept as the Bevy Remote Protocol, arrived at independently |
| `scene`, `world_serialization` | `.scene` / `.prefab` RON | Theirs is BSN — code-first, **no `.bsn` asset loader yet**. Ours is already file-first |
| `time` | `kooch_core::time` | |
| `transform` | `kooch_ecs::hierarchy` | 🔴 Theirs got **11× via dirty-bit skipping** (0.16). Ours propagates unconditionally |
| `window`, `winit` | `kooch_window` | |
| `diagnostic` | `kooch_core::frame_metrics` + editor perf HUD | |
| `feathers` | The editor's egui panels | Theirs is a widget toolkit *for editors*; worth reading for what an editor needs |
| `solari` | — | Deliberately not: needs hardware RT. Ours is surfel + voxel (#450) |
| `sprite`, `sprite_render`, `a11y`, `platform`, `clipboard`, `utils` | — | Not on this engine's path, or trivial |

## 🔴 Missing, and it hurts the goal

| Bevy | What it is | Why it matters for universes | Issue |
|---|---|---|---|
| **`tasks`** | Task pools — a real async executor for background work | **Nothing exists here.** We have loose `thread::spawn` in `frame_pacing`, `runner` and the editor's remote session, and no pool. Streaming a planet means loading chunks, meshes and textures without stalling the frame. There is no mechanism for that today | **none — needs one** |
| **`state`** | App-wide finite state machine (`States`, sub-states, state-scoped entities) | **Nothing exists here.** Menu → loading → playing → paused. Also the consumer that `ActionMap` bulk enable/disable is waiting for. State-scoped entities despawn on transition, which is exactly what unloading a level is | **none — needs one** |
| **`text`** | Font loading, shaping, layout | No runtime text at all. The editor has egui; a game has nothing. A score readout is impossible today | #96 / #280 |
| **`ui`, `ui_render`, `ui_widgets`** | Runtime UI | Same. They use taffy + **parley** (moved off cosmic-text in 0.19) | #96 / #280 |
| **`post_process`** | Bloom, tonemapping, DOF, motion blur | Auto exposure and tonemapping stop being cosmetic at planetary scale: sunlit surface to night side spans orders of magnitude | #254 |
| **`anti_alias`** | SMAA / TAA / FXAA | Meshlet-dense geometry at distance aliases badly. This is a *far-field* problem, not a polish problem | #254 |
| **`color`** | Colour spaces — sRGB, linear, OkLab, conversions | We touch sRGB in the texture loader and the GPU context, and have no colour type. Wrong-space blending is invisible until it is everywhere | **none** |
| **`dev_tools`** | Debug overlays, **and the infinite grid** (first-party in 0.19) | A shader-drawn ground plane with distance fade: no geometry, works at any scale. The editor viewport has no ground reference at all | **none** |
| **`settings`** | Saving and loading user settings files | The editor has `EditorConfig` ad hoc; a game has nothing | **none** |
| **`animation`** | | #92 / #717 already cover it | #92 / #717 |

## What this changes

**Two of these are not on any roadmap and both block the goal:**

- **Task pool.** A planet that streams has to load off the frame. Every
  approach to large worlds assumes background work. We have threads spawned
  by hand in three places and no pool, no cancellation, no priority.
- **App state machine.** Every level transition, pause menu and loading
  screen needs it, and its absence is why `ActionMap`'s bulk enable/disable
  has no consumer.

**Two more are cheap and change how the editor feels**, which was asked
for directly: the **infinite grid** (`bevy_dev_tools::infinite_grid`,
MIT/Apache-2.0 as the original crate) and reading **`feathers`** for what
an editor widget set actually needs.

**One is a silent correctness hazard**: no colour type. Everything works
until two things blend in the wrong space, and then everything is subtly
wrong at once.

## Method note

The first sweep read release notes and missed everything that was not
*new* — the grid, the task pool, the state machine, colour. Notes are a
changelog; the module index is an inventory. Read both.
