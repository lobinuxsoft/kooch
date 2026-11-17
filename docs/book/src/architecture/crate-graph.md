# Crate Graph

Kóoch is a Cargo workspace of **27 crates**: 26 under `crates/` — two of them
in-tree forks — plus the top-level `kooch` facade. The structure is
intentionally fine-grained: each subsystem lives in its own crate so that
downstream crates only depend on what they actually need. This keeps compile
times low when iterating on a single subsystem and makes the dependency
surface auditable at a glance.

## Layers at a glance

They sit in **ten** layers. Each layer may only depend on layers below it.

The layer of a crate is its **longest path to a crate with no internal
dependencies**. That is a property of `Cargo.toml`, not an opinion, so it is
read out of the workspace rather than assigned:

```sh
python3 .github/scripts/crate_layers.py          # the two tables below
python3 .github/scripts/crate_layers.py --check   # or just: did they drift?
```

🔴 **This page has drifted twice, and the drift is the argument for that
script.** The second time it claimed nine layers over 20 crates and did not
mention `kooch_ui`, `kooch_video`, `kooch_blockmesh`, `kooch_character` or
the two forks — because the check only read names starting with `kooch`
and the forks were invisible to it. Run `--check` after any `Cargo.toml`
edit that adds an internal dependency.

| Layer | Crates | Role |
|-------|--------|------|
| **L0 · foundation** | `kooch_plugin_api`, `kooch_ecs_macros`, `kooch_pack`, `kooch_video`, `egui-snarl`, `vello_svg` | No internal deps. Type vocabulary, proc-macros, the shipped-asset container, the video decoder, and the two forks. |
| **L1 · plugin GPU half** | `kooch_plugin_render` | The GPU half of the plugin API, on top of `kooch_plugin_api`. |
| **L2 · core** | `kooch_core` | `App`, `Plugin`, `Schedule`, `Resources`, `GpuContext`, the asset server. |
| **L3 · primitives** | `kooch_ecs`, `kooch_window`, `kooch_audio` | ECS, windowing, audio. Depend on `kooch_core` only. |
| **L4 · domain** | `kooch_input`, `kooch_lighting`, `kooch_physics`, `kooch_remote`, `kooch_world` | Built on the ECS. Input actions, lighting data, simulation, remote protocol, scene organisation. |
| **L5 · built on domain** | `kooch_render`, `kooch_gravity`, `kooch_ui` | The renderer needs Inti's shading model; gravity needs the solver; UI needs input. |
| **L6 · built on L5** | `kooch_gizmos`, `kooch_camera`, `kooch_character`, `kooch_blockmesh` | Gizmos and blocks submit geometry through the renderer; the camera and the character ask the gravity field which way is up. |
| **L7 · gizmo interaction** | `kooch_gizmos_handles` | Draggable handles on top of gizmo drawing. |
| **L8 · editor** | `kooch_editor_core` | Editor logic as a library. Depends on **18** internal crates — the widest surface in the workspace. |
| **L9 · binary + facade** | `kooch_editor`, `kooch` | The editor `main()`, and the facade user projects depend on. |

Three of those placements are worth a sentence, because none is where a
reader would guess:

- **`kooch_camera` is L6, not L4**, because a `VirtualCamera` asks
  `kooch_gravity::gravity_up` which way up is. On a planet, up is not `+Y`,
  and a camera rig that assumed it would roll over at the equator. Without
  `kooch_gravity` compiled in, the mode falls back to world up.
- **`kooch_core` depends on `kooch_pack`**, which is why the container crate
  is foundation rather than tooling: the asset server opens `.kpack` files,
  so the format has to sit under the crate that reads them.
- **`kooch_video` is foundation**: it decodes to YUV planes and knows
  nothing about the GPU, so both `kooch_ui` (which picks a frame) and
  `kooch_render` (which converts it) can sit on it.

## Inter-layer flow

The arrow direction reads as *"A depends on B"*. Within a layer, crates
are siblings.

```mermaid
flowchart TD
    L9["L9 · binary + facade<br/>kooch_editor · kooch"]
    L8["L8 · editor<br/>kooch_editor_core"]
    L7["L7 · gizmo interaction<br/>kooch_gizmos_handles"]
    L6["L6 · built on L5<br/>kooch_gizmos · kooch_camera<br/>kooch_character · kooch_blockmesh"]
    L5["L5 · built on domain<br/>kooch_render · kooch_gravity · kooch_ui"]
    L4["L4 · domain<br/>kooch_input · kooch_lighting · kooch_physics<br/>kooch_remote · kooch_world"]
    L3["L3 · primitives<br/>kooch_ecs · kooch_window · kooch_audio"]
    L2["L2 · core<br/>kooch_core"]
    L1["L1 · plugin GPU half<br/>kooch_plugin_render"]
    L0["L0 · foundation<br/>kooch_plugin_api · kooch_ecs_macros · kooch_pack<br/>kooch_video · egui-snarl · vello_svg"]

    L9 --> L8
    L8 --> L7
    L8 --> L6
    L8 --> L5
    L8 --> L4
    L8 --> L3
    L8 --> L0
    L7 --> L6
    L6 --> L5
    L6 --> L4
    L5 --> L4
    L5 --> L0
    L4 --> L3
    L3 --> L2
    L2 --> L1
    L2 --> L0
    L1 --> L0
```

## Detailed dependency table

Per-crate internal dependencies (external deps like `wgpu`, `winit`, etc.
are omitted).

| Crate | Depends on |
|-------|-----------|
| `egui-snarl` | — |
| `vello_svg` | — |
| `kooch_ecs_macros` | — |
| `kooch_pack` | — |
| `kooch_plugin_api` | — |
| `kooch_video` | — |
| `kooch_plugin_render` | `kooch_plugin_api` (+ wgpu) — the GPU half of the plugin API, linked only by a plugin that draws (#392) |
| `kooch_core` | `kooch_pack`, `kooch_plugin_api`, `kooch_plugin_render` |
| `kooch_audio` | `kooch_core` |
| `kooch_window` | `kooch_core` |
| `kooch_ecs` | `kooch_core`, `kooch_ecs_macros`, `kooch_plugin_api` |
| `kooch_input` | `kooch_core`, `kooch_ecs`, `kooch_window` |
| `kooch_lighting` | `kooch_core`, `kooch_ecs` |
| `kooch_physics` | `kooch_core`, `kooch_ecs` |
| `kooch_remote` | `kooch_core`, `kooch_ecs` |
| `kooch_world` | `kooch_core`, `kooch_ecs` |
| `kooch_gravity` | `kooch_core`, `kooch_ecs`, `kooch_physics` |
| `kooch_render` | `kooch_core`, `kooch_ecs`, `kooch_lighting`, `kooch_video`, `vello_svg` |
| `kooch_ui` | `kooch_core`, `kooch_ecs`, `kooch_input`, `kooch_video` |
| `kooch_blockmesh` | `kooch_core`, `kooch_ecs`, `kooch_ecs_macros`, `kooch_physics`, `kooch_render` |
| `kooch_camera` | `kooch_core`, `kooch_ecs`, `kooch_gravity`, `kooch_input`, `kooch_physics` |
| `kooch_character` | `kooch_core`, `kooch_ecs`, `kooch_gravity`, `kooch_physics` |
| `kooch_gizmos` | `kooch_core`, `kooch_ecs`, `kooch_render` |
| `kooch_gizmos_handles` | `kooch_gizmos` |
| `kooch_editor_core` | `egui-snarl`, `kooch_blockmesh`, `kooch_camera`, `kooch_character`, `kooch_core`, `kooch_ecs`, `kooch_gizmos`, `kooch_gizmos_handles`, `kooch_gravity`, `kooch_input`, `kooch_lighting`, `kooch_pack`, `kooch_physics`, `kooch_remote`, `kooch_render`, `kooch_ui`, `kooch_window`, `kooch_world` |
| `kooch_editor` | `kooch_core`, `kooch_ecs`, `kooch_editor_core`, `kooch_render`, `kooch_window`, `kooch_world` |
| `kooch` | `kooch_core`, `kooch_ecs` (always); `kooch_audio`, `kooch_blockmesh`, `kooch_camera`, `kooch_character`, `kooch_editor_core`, `kooch_gizmos`, `kooch_gravity`, `kooch_input`, `kooch_lighting`, `kooch_physics`, `kooch_plugin_api`, `kooch_remote`, `kooch_render`, `kooch_ui`, `kooch_window`, `kooch_world` (optional, feature-gated) |

⚠️ **`examples/example_plugin` and `examples/example_post_process` are
workspace members and are not among the 27.** They are members so that
`cargo test` builds them — a plugin that stops compiling is a broken ABI,
and finding that out from a user is late.

## Crate roles

### Foundation (L0, L1)

Crates with **no internal dependencies** (and `kooch_plugin_render`, which
has only `kooch_plugin_api`). They can be built in isolation and form the
type vocabulary the rest of the engine uses.

| Crate | Role |
|-------|------|
| `kooch_plugin_api` | Stable ABI types for dynamic plugins (loaded via `libloading`), including the `KoochPlugin` trait a project implements. Lives below `kooch_core` so plugins compiled against an old engine can still be probed. |
| `kooch_plugin_render` | The GPU half of the plugin API, linked only by a plugin that draws (#392). |
| `kooch_ecs_macros` | Procedural macros for the ECS: `#[derive(Reflect)]`, `#[derive(Component)]`. Standalone proc-macro crate. |
| `kooch_pack` | `.kpack` — the zstd + AES-256-GCM container a shipped game reads its assets out of. Foundation rather than tooling because **`kooch_core`'s asset server opens them**, so the format sits under the crate that reads it. ⚠️ The key ships inside the binary, so it is a deterrent and not protection — Godot's own docs say the same about theirs. |
| `kooch_video` | WebM/AV1 decoding (#1793): dav1d on its own thread, frames handed over as YUV planes for the GPU to convert where they land. |
| `egui-snarl` | In-tree fork of the egui node-graph crate (#1159): the published one has no selection setter and no node sizes (#1211). Used by the shader graph. |
| `vello_svg` | In-tree fork of the SVG-to-vello crate (#1865), until upstream reaches vello 0.11. |

### Core (L2)

| Crate | Role |
|-------|------|
| `kooch_core` | `App`, `Plugin`, `PluginGroup`, `Stage`, `Schedule`, `Resources`, `Time`, `GpuContext`, event system, asset server, pipeline cache, power profile detection, and `scene_paths` — the file names three crates have to agree on. The minimum any Kóoch binary needs. |

### Primitives (L3)

Small purpose-built crates that depend on `kooch_core` and nothing else.

| Crate | Role |
|-------|------|
| `kooch_ecs` | The ECS itself: archetype storage, `Entity`/`Component` traits, `Query`, `Reflect`, `SceneDocument`/`SceneManager`, hierarchy, transforms, built-in components (Transform, Name, PerspectiveCamera, OrthographicCamera, Mesh, lights, sky, etc). |
| `kooch_window` | Winit integration, `WindowPlugin`, surface configuration, raw event dispatch. |
| `kooch_audio` | Kira-based audio playback. Sits here rather than beside the ECS crates because it does not depend on the ECS: **there is still no `AudioSource` component** (#1645), so there is nothing to author. |

### Domain (L4)

Crates built directly on the ECS.

| Crate | Role |
|-------|------|
| `kooch_input` | Gamepad / keyboard / mouse abstraction. On the ECS since an **action became an asset** (#55): a component points at an `.inputaction` by guid, so nothing in gameplay names an action. |
| `kooch_lighting` | **Inti** — the shading model, the GPU light record, extraction, exposure and ambient. The light *components* live in `kooch_ecs` beside every other component; what lives here is everything that turns them into pixels. See [Lighting](./lighting.md). |
| `kooch_physics` | Physics simulation. Rapier is the backend, behind `kooch_physics`'s own types. |
| `kooch_remote` | The local-socket protocol that lets the standalone editor drive a running project's ECS. |
| `kooch_world` | Scene/world organisation, chunk streaming and activation. |

### Built on domain (L5)

| Crate | Role |
|-------|------|
| `kooch_render` | The GPU work: meshlet pipeline (cull, visibility buffer, shading, Hi-Z), shadows, the froxel grid's consumer side, `RenderPlugin`, materials, glTF loading, the UI and video draw. Above `kooch_lighting` because **a renderer without a shading model paints normals** — which is literally what this one did until #441. See [Render Pipeline](./render-pipeline.md). |
| `kooch_gravity` | Multi-gravity system (Mario Galaxy-style fields). Needs `kooch_physics` to apply forces. |
| `kooch_ui` | In-game UI (#96): one entity per `.kxml` document, its nodes as SoA over a `TaffyTree` rather than as entities. Overlay and world-space share one pipeline. See [UI](../guide/ui.md). |

### Built on L5 (L6)

| Crate | Role |
|-------|------|
| `kooch_gizmos` | Immediate-mode gizmo drawing. Needs `kooch_render` to submit geometry. |
| `kooch_camera` | Camera components and `VirtualCamera` (follow / look-at with damping). **Needs `kooch_gravity`**, not the renderer: a rig asks `gravity_up` which way up is, because on a planet up is not `+Y` and a camera that assumed it would roll over at the equator. |
| `kooch_character` | A character that stands on ground wherever it is: a dynamic capsule floating on a damped spring over a swept sphere, oriented to the local up. |
| `kooch_blockmesh` | `BlockMesh`, the authoring mesh the editor's block tool edits; regenerates a render mesh and a collider from it. |

### Gizmo interaction (L7)

| Crate | Role |
|-------|------|
| `kooch_gizmos_handles` | Draggable translate / rotate / scale / plane handles, with snapping and Local/World modes. Split from `kooch_gizmos` because drawing a gizmo and interacting with one are different problems. |

### Editor (L8)

| Crate | Role |
|-------|------|
| `kooch_editor_core` | All editor logic as a library: panels (hierarchy, inspector, viewport, console, asset browser, profiler), undo/redo, project state and manifest, scene and prefab save/load, play/stop, the launch screen, and the build presets that produce a `.kpack`. Used by the editor binary AND callable as a plugin from custom hosts. |

### Binary and facade (L9)

| Crate | Role |
|-------|------|
| `kooch_editor` | The editor `main()`. Imports `kooch_editor_core` and runs an `App` with the editor plugins wired. |
| `kooch` | Top-level facade that re-exports the others under one name and defines `DefaultPlugins` (Bevy-style PluginGroup). User project crates depend on `kooch` rather than picking subcrates directly. Cargo features (`window`, `render`, `audio`, `ui`, `editor`, `dynamic`...) gate which sub-crates pull in. |

## Layering rules

> **Important:** Lower layers must not depend on higher layers. If you find
> yourself wanting `kooch_core` to know about `kooch_render`, you have an
> inversion. Common fixes: introduce a trait at the lower layer and
> implement it at the higher layer, or pass behavior in via a generic /
> closure.

The dependency graph is acyclic by construction — Cargo refuses a cycle, so
that half needs no enforcing. What *is* only enforced by review is whether a
new edge is the *right* edge: `kooch_camera → kooch_gravity` is a real
dependency and moved the crate two layers, and nothing objected.

`crate_layers.py --check` catches the documentation drifting from the graph.
It does not catch the graph drifting from the design.

## Why so many crates?

Three reasons:

1. **Compile-time isolation.** Iterating on `kooch_render` does not
   recompile `kooch_ecs` or `kooch_window`. Cargo's incremental compilation
   benefits from real crate boundaries far more than from module boundaries
   inside one giant crate.

2. **Feature gating.** The facade can compose user-facing builds (a
   headless server build skips `kooch_render` and `kooch_window`; an editor
   build pulls in `kooch_editor_core`). Single-crate builds cannot do this
   ergonomically.

3. **Reasoning surface.** Knowing that `kooch_world` cannot accidentally
   reach into `kooch_render` because Cargo enforces it makes refactors safer
   than relying on lint rules.

Tradeoff: more `Cargo.toml` files to maintain, more `pub use` re-exports
when types need to cross crate boundaries, slightly higher first-build time.
For an engine in early development the compile-time win outweighs the
ergonomic cost.

## Adding a new crate

1. Create `crates/kooch_yourthing/` with `Cargo.toml` extending
   `workspace.package` fields and `Cargo.toml` `[workspace.dependencies]` for
   external deps.
2. Add it to the `members` array in the root `Cargo.toml`.
3. Add it under `[workspace.dependencies]` so other crates can depend on
   it via `workspace = true`.
4. If it's user-facing, re-export from `kooch::*` and add a feature
   flag to the top-level `Cargo.toml` `[features]` section.
5. Document its role in this page, then run
   `python3 .github/scripts/crate_layers.py --check` — a new crate usually
   moves somebody else's layer, and that is the part nobody remembers.
