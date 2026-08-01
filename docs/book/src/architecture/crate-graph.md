# Crate Graph

Kóoch is a Cargo workspace of 18 internal crates plus one top-level facade
crate. The structure is intentionally fine-grained: each subsystem lives in
its own crate so that downstream crates only depend on what they actually
need. This keeps compile times low when iterating on a single subsystem and
makes the dependency surface auditable at a glance.

## Layers at a glance

The 18 internal crates (plus the top-level `kooch` facade) sit in
seven layers. Each layer may only depend on layers below it.

| Layer | Crates | Role |
|-------|--------|------|
| **L0 · foundation** | `kooch_plugin_api`, `kooch_ecs_macros` | No internal deps. Type vocabulary + proc-macros. |
| **L1 · core** | `kooch_core` | `App`, `Plugin`, `Schedule`, `Resources`, `GpuContext`. |
| **L2 · primitives** | `kooch_ecs`, `kooch_window`, `kooch_input` | ECS, windowing, input. |
| **L3 · domain** | `kooch_sdf`, `kooch_lighting`, `kooch_world`, `kooch_audio`, `kooch_scripting` | Domain types (SDFs, lights, audio, scripting), no GPU work. |
| **L4 · subsystems** | `kooch_render`, `kooch_physics`, `kooch_gravity` | GPU-heavy systems built on L0–L3. |
| **L5 · editor** | `kooch_editor_core` | Editor logic as a library. |
| **L6 · binary** | `kooch_editor` | The editor `main()`. |
| **facade** | `kooch` | Top-level re-exports + `DefaultPlugins`. User projects depend on this. |

## Inter-layer flow

The arrow direction reads as *"A depends on B"*. Within a layer, crates
are siblings.

```mermaid
flowchart TD
    L6["L6 · binary<br/>kooch_editor"]
    L5["L5 · editor<br/>kooch_editor_core"]
    L4["L4 · subsystems<br/>kooch_render · kooch_physics · kooch_gravity"]
    L3["L3 · domain<br/>kooch_sdf · kooch_lighting · kooch_world · kooch_audio · kooch_scripting"]
    L2["L2 · primitives<br/>kooch_ecs · kooch_window · kooch_input"]
    L1["L1 · core<br/>kooch_core"]
    L0["L0 · foundation<br/>kooch_plugin_api · kooch_ecs_macros"]
    FACADE["facade<br/>kooch"]

    L6 --> L5
    L5 --> L4
    L5 --> L2
    L4 --> L3
    L4 --> L2
    L3 --> L2
    L2 --> L1
    L1 --> L0
    FACADE -.optional.-> L4
    FACADE -.optional.-> L5
    FACADE --> L2
```

## Detailed dependency table

Per-crate internal dependencies (external deps like `wgpu`, `winit`, etc.
are omitted).

| Crate | Depends on |
|-------|-----------|
| `kooch_plugin_api` | — |
| `kooch_ecs_macros` | — |
| `kooch_core` | `kooch_plugin_api` |
| `kooch_ecs` | `kooch_core`, `kooch_ecs_macros`, `kooch_plugin_api` |
| `kooch_window` | `kooch_core` |
| `kooch_input` | `kooch_core` |
| `kooch_sdf` | `kooch_core`, `kooch_ecs` |
| `kooch_lighting` | `kooch_core`, `kooch_ecs` |
| `kooch_world` | `kooch_core`, `kooch_ecs` |
| `kooch_audio` | `kooch_core`, `kooch_ecs` |
| `kooch_scripting` | `kooch_core`, `kooch_ecs` |
| `kooch_render` | `kooch_core`, `kooch_ecs`, `kooch_sdf` |
| `kooch_physics` | `kooch_core`, `kooch_ecs`, `kooch_sdf` |
| `kooch_gravity` | `kooch_core`, `kooch_ecs`, `kooch_sdf` |
| `kooch_editor_core` | `kooch_core`, `kooch_ecs`, `kooch_render`, `kooch_window` |
| `kooch_editor` | `kooch_core`, `kooch_ecs`, `kooch_editor_core`, `kooch_window` |
| `kooch` | `kooch_core`, `kooch_ecs` (always); `kooch_window`, `kooch_render`, `kooch_editor_core` (feature-gated) |

## Crate roles

### Foundation (L0)

Crates with **no internal dependencies**. They can be built in isolation and
form the type vocabulary the rest of the engine uses.

| Crate | Role |
|-------|------|
| `kooch_plugin_api` | Stable ABI types for dynamic plugins (loaded via `libloading`). Lives below `kooch_core` so plugins compiled against an old engine can still be probed. |
| `kooch_ecs_macros` | Procedural macros for the ECS: `#[derive(Reflect)]`, `#[derive(Component)]`. Standalone proc-macro crate. |

### Core (L1)

| Crate | Role |
|-------|------|
| `kooch_core` | `App`, `Plugin`, `PluginGroup`, `Stage`, `Schedule`, `Resources`, `Time`, `GpuContext`, event system, pipeline cache, power profile detection. The minimum any Kóoch binary needs. |

### Primitives (L2)

Small purpose-built crates the rest of the engine builds on.

| Crate | Role |
|-------|------|
| `kooch_ecs` | The ECS itself: archetype storage, `Entity`/`Component` traits, `Query`, `Reflect`, `SceneDocument`/`SceneManager`, hierarchy, transforms, built-in components (Transform, Name, PerspectiveCamera, OrthographicCamera, Mesh, SDF primitives, lights, sky, etc). |
| `kooch_window` | Winit integration, `WindowPlugin`, surface configuration, raw event dispatch. |
| `kooch_input` | Gamepad / keyboard / mouse abstraction. |

### Domain (L3)

Crates that introduce domain types but no rendering or simulation logic.

| Crate | Role |
|-------|------|
| `kooch_sdf` | Pure SDF math primitives (sphere, box, capsule, etc.) used by ray-march renderer and physics queries. |
| `kooch_lighting` | Light components (DirectionalLight, PointLight, SpotLight). |
| `kooch_world` | Scene/world organization helpers. Currently small. |
| `kooch_audio` | Kira-based audio playback with spatial support. |
| `kooch_scripting` | Rhai-based scripting integration. |

### Subsystems (L4)

| Crate | Role |
|-------|------|
| `kooch_render` | The actual GPU work: `RayMarchRenderer`, `MeshPassRenderer`, `SkyRenderPass`, `RenderPlugin` (orchestrator), `RayMarchPlugin` (standalone demo path). |
| `kooch_physics` | Physics simulation. |
| `kooch_gravity` | Multi-gravity system (Mario-Galaxy style). |

### Editor (L5)

| Crate | Role |
|-------|------|
| `kooch_editor_core` | All editor logic as a library: panels (hierarchy, inspector, viewport), undo/redo, project state, scene save/load actions, play/stop. Used by the editor binary AND callable as a plugin from custom hosts. |

### Binary (L6)

| Crate | Role |
|-------|------|
| `kooch_editor` | The editor `main()`. Imports `kooch_editor_core` and runs an `App` with the editor plugins wired. |

### Facade

| Crate | Role |
|-------|------|
| `kooch` | Top-level facade that re-exports the others under one name and defines `DefaultPlugins` (Bevy-style PluginGroup). User project crates depend on `kooch` rather than picking subcrates directly. Cargo features (`window`, `render`, `audio`, `editor`, `dynamic`...) gate which sub-crates pull in. |

## Layering rules

> **Important:** Lower layers must not depend on higher layers. If you find
> yourself wanting `kooch_core` to know about `kooch_render`, you have an
> inversion. Common fixes: introduce a trait at the lower layer and
> implement it at the higher layer, or pass behavior in via a generic /
> closure.

The dependency graph is acyclic by construction. CI does not enforce this
yet — manual review during code review.

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
5. Document its role in this page.
