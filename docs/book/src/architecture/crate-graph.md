# Crate Graph

Kóoch is a Cargo workspace of **20 crates**: 19 under `crates/` plus the
top-level `kooch` facade. The structure is intentionally fine-grained: each
subsystem lives in its own crate so that downstream crates only depend on
what they actually need. This keeps compile times low when iterating on a
single subsystem and makes the dependency surface auditable at a glance.

## Layers at a glance

They sit in **nine** layers. Each layer may only depend on layers below it.

The layer of a crate is its **longest path to a crate with no internal
dependencies**. That is a property of `Cargo.toml`, not an opinion, so it is
read out of the workspace rather than assigned:

```sh
python3 .github/scripts/crate_layers.py          # the two tables below
python3 .github/scripts/crate_layers.py --check   # or just: did they drift?
```

🔴 **This page had drifted, and the drift is the argument for that script.**
It claimed eight layers over 18 crates, put `kooch_input` and `kooch_camera`
two and three layers below where they are, listed `kooch_render` in *two*
tables at once, and did not mention `kooch_pack` at all — while asserting
that the layers move "whether or not anyone updates this page". Derived once
by hand is not derived.

| Layer | Crates | Role |
|-------|--------|------|
| **L0 · foundation** | `kooch_plugin_api`, `kooch_ecs_macros`, `kooch_pack` | No internal deps. Type vocabulary, proc-macros, the shipped-asset container. |
| **L1 · core** | `kooch_core` | `App`, `Plugin`, `Schedule`, `Resources`, `GpuContext`, the asset server. |
| **L2 · primitives** | `kooch_ecs`, `kooch_window`, `kooch_audio` | ECS, windowing, audio. Depend on `kooch_core` only. |
| **L3 · domain** | `kooch_input`, `kooch_lighting`, `kooch_physics`, `kooch_remote`, `kooch_world` | Built on the ECS. Input actions, lighting data, simulation, remote protocol, scene organisation. |
| **L4 · built on domain** | `kooch_render`, `kooch_gravity` | The renderer needs Inti's shading model; gravity needs the solver. |
| **L5 · built on the renderer** | `kooch_gizmos`, `kooch_camera` | Gizmos submit geometry through the renderer; the camera asks the gravity field which way is up. |
| **L6 · gizmo interaction** | `kooch_gizmos_handles` | Draggable handles on top of gizmo drawing. |
| **L7 · editor** | `kooch_editor_core` | Editor logic as a library. Depends on **14** internal crates — the widest surface in the workspace. |
| **L8 · binary + facade** | `kooch_editor`, `kooch` | The editor `main()`, and the facade user projects depend on. |

Two of those placements are worth a sentence, because neither is where a
reader would guess:

- **`kooch_camera` is L5, not L3**, because a `VirtualCamera` asks
  `kooch_gravity::gravity_up` which way up is. On a planet, up is not `+Y`,
  and a camera rig that assumed it would roll over at the equator. Without
  `kooch_gravity` compiled in, the mode falls back to world up.
- **`kooch_core` depends on `kooch_pack`**, which is why the container crate
  is foundation rather than tooling: the asset server opens `.kpack` files,
  so the format has to sit under the crate that reads them.

## Inter-layer flow

The arrow direction reads as *"A depends on B"*. Within a layer, crates
are siblings.

```mermaid
flowchart TD
    L8["L8 · binary + facade<br/>kooch_editor · kooch"]
    L7["L7 · editor<br/>kooch_editor_core"]
    L6["L6 · gizmo interaction<br/>kooch_gizmos_handles"]
    L5["L5 · built on the renderer<br/>kooch_gizmos · kooch_camera"]
    L4["L4 · built on domain<br/>kooch_render · kooch_gravity"]
    L3["L3 · domain<br/>kooch_input · kooch_lighting · kooch_physics<br/>kooch_remote · kooch_world"]
    L2["L2 · primitives<br/>kooch_ecs · kooch_window · kooch_audio"]
    L1["L1 · core<br/>kooch_core"]
    L0["L0 · foundation<br/>kooch_plugin_api · kooch_ecs_macros · kooch_pack"]

    L8 --> L7
    L7 --> L6
    L7 --> L5
    L7 --> L4
    L7 --> L3
    L7 --> L2
    L6 --> L5
    L5 --> L4
    L5 --> L3
    L4 --> L3
    L3 --> L2
    L2 --> L1
    L1 --> L0
```

## Detailed dependency table

Per-crate internal dependencies (external deps like `wgpu`, `winit`, etc.
are omitted).

| Crate | Depends on |
|-------|-----------|
| `kooch_ecs_macros` | — |
| `kooch_pack` | — |
| `kooch_plugin_api` | — |
| `kooch_core` | `kooch_pack`, `kooch_plugin_api` |
| `kooch_audio` | `kooch_core` |
| `kooch_window` | `kooch_core` |
| `kooch_ecs` | `kooch_core`, `kooch_ecs_macros`, `kooch_plugin_api` |
| `kooch_input` | `kooch_core`, `kooch_ecs` |
| `kooch_lighting` | `kooch_core`, `kooch_ecs` |
| `kooch_physics` | `kooch_core`, `kooch_ecs` |
| `kooch_remote` | `kooch_core`, `kooch_ecs` |
| `kooch_world` | `kooch_core`, `kooch_ecs` |
| `kooch_gravity` | `kooch_core`, `kooch_ecs`, `kooch_physics` |
| `kooch_render` | `kooch_core`, `kooch_ecs`, `kooch_lighting` |
| `kooch_camera` | `kooch_core`, `kooch_ecs`, `kooch_gravity` |
| `kooch_gizmos` | `kooch_core`, `kooch_ecs`, `kooch_render` |
| `kooch_gizmos_handles` | `kooch_gizmos` |
| `kooch_editor_core` | `kooch_camera`, `kooch_core`, `kooch_ecs`, `kooch_gizmos`, `kooch_gizmos_handles`, `kooch_gravity`, `kooch_input`, `kooch_lighting`, `kooch_pack`, `kooch_physics`, `kooch_remote`, `kooch_render`, `kooch_window`, `kooch_world` |
| `kooch_editor` | `kooch_core`, `kooch_ecs`, `kooch_editor_core`, `kooch_render`, `kooch_window`, `kooch_world` |
| `kooch` | `kooch_core`, `kooch_ecs` (always); `kooch_audio`, `kooch_camera`, `kooch_editor_core`, `kooch_gizmos`, `kooch_gravity`, `kooch_input`, `kooch_lighting`, `kooch_physics`, `kooch_plugin_api`, `kooch_remote`, `kooch_render`, `kooch_window`, `kooch_world` (optional, feature-gated) |

⚠️ **`examples/example_plugin` is a workspace member and is not one of the
20.** It is a member so that `cargo test` builds it — a plugin that stops
compiling is a broken ABI, and finding that out from a user is late.

## Crate roles

### Foundation (L0)

Crates with **no internal dependencies**. They can be built in isolation and
form the type vocabulary the rest of the engine uses.

| Crate | Role |
|-------|------|
| `kooch_plugin_api` | Stable ABI types for dynamic plugins (loaded via `libloading`), including the `KoochPlugin` trait a project implements. Lives below `kooch_core` so plugins compiled against an old engine can still be probed. |
| `kooch_ecs_macros` | Procedural macros for the ECS: `#[derive(Reflect)]`, `#[derive(Component)]`. Standalone proc-macro crate. |
| `kooch_pack` | `.kpack` — the zstd + AES-256-GCM container a shipped game reads its assets out of. Foundation rather than tooling because **`kooch_core`'s asset server opens them**, so the format sits under the crate that reads it. ⚠️ The key ships inside the binary, so it is a deterrent and not protection — Godot's own docs say the same about theirs. |

### Core (L1)

| Crate | Role |
|-------|------|
| `kooch_core` | `App`, `Plugin`, `PluginGroup`, `Stage`, `Schedule`, `Resources`, `Time`, `GpuContext`, event system, asset server, pipeline cache, power profile detection, and `scene_paths` — the file names three crates have to agree on. The minimum any Kóoch binary needs. |

### Primitives (L2)

Small purpose-built crates that depend on `kooch_core` and nothing else.

| Crate | Role |
|-------|------|
| `kooch_ecs` | The ECS itself: archetype storage, `Entity`/`Component` traits, `Query`, `Reflect`, `SceneDocument`/`SceneManager`, hierarchy, transforms, built-in components (Transform, Name, PerspectiveCamera, OrthographicCamera, Mesh, lights, sky, etc). |
| `kooch_window` | Winit integration, `WindowPlugin`, surface configuration, raw event dispatch. |
| `kooch_audio` | Kira-based audio playback. Sits here rather than beside the ECS crates because it does not depend on the ECS: **there is still no `AudioSource` component** (#63), so there is nothing to author. |

### Domain (L3)

Crates built directly on the ECS.

| Crate | Role |
|-------|------|
| `kooch_input` | Gamepad / keyboard / mouse abstraction. On the ECS since an **action became an asset** (#55): a component points at an `.inputaction` by guid, so nothing in gameplay names an action. |
| `kooch_lighting` | **Inti** — the shading model, the GPU light record, extraction, exposure and ambient. The light *components* live in `kooch_ecs` beside every other component; what lives here is everything that turns them into pixels. See [Lighting](./lighting.md). |
| `kooch_physics` | Physics simulation. Rapier is the backend, behind `kooch_physics`'s own types. |
| `kooch_remote` | The local-socket protocol that lets the standalone editor drive a running project's ECS. |
| `kooch_world` | Scene/world organisation, chunk streaming and activation. |

### Built on domain (L4)

| Crate | Role |
|-------|------|
| `kooch_render` | The GPU work: meshlet pipeline (cull, visibility buffer, shading, Hi-Z), shadows, the froxel grid's consumer side, `RenderPlugin`, materials, glTF loading. Above `kooch_lighting` because **a renderer without a shading model paints normals** — which is literally what this one did until #441. See [Render Pipeline](./render-pipeline.md). |
| `kooch_gravity` | Multi-gravity system (Mario Galaxy-style fields). Needs `kooch_physics` to apply forces. |

### Built on the renderer (L5)

| Crate | Role |
|-------|------|
| `kooch_gizmos` | Immediate-mode gizmo drawing. Needs `kooch_render` to submit geometry. |
| `kooch_camera` | Camera components and `VirtualCamera` (follow / look-at with damping). **Needs `kooch_gravity`**, not the renderer: a rig asks `gravity_up` which way up is, because on a planet up is not `+Y` and a camera that assumed it would roll over at the equator. |

### Gizmo interaction (L6)

| Crate | Role |
|-------|------|
| `kooch_gizmos_handles` | Draggable translate / rotate / scale / plane handles, with snapping and Local/World modes. Split from `kooch_gizmos` because drawing a gizmo and interacting with one are different problems. |

### Editor (L7)

| Crate | Role |
|-------|------|
| `kooch_editor_core` | All editor logic as a library: panels (hierarchy, inspector, viewport, console, asset browser, profiler), undo/redo, project state and manifest, scene and prefab save/load, play/stop, the launch screen, and the build presets that produce a `.kpack`. Used by the editor binary AND callable as a plugin from custom hosts. |

### Binary and facade (L8)

| Crate | Role |
|-------|------|
| `kooch_editor` | The editor `main()`. Imports `kooch_editor_core` and runs an `App` with the editor plugins wired. |
| `kooch` | Top-level facade that re-exports the others under one name and defines `DefaultPlugins` (Bevy-style PluginGroup). User project crates depend on `kooch` rather than picking subcrates directly. Cargo features (`window`, `render`, `audio`, `editor`, `dynamic`...) gate which sub-crates pull in. |

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
