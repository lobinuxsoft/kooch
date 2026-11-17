# Introduction

**Kóoch** is a GPU-driven game engine written in Rust, with an editor.

The rendering path is a **Nanite-style GPU-driven meshlet pipeline**: the hot loop runs in
compute on the GPU, and the CPU only coordinates. The ECS stays on the CPU — that is a
deliberate, settled decision, not a stage on the way to something else. What goes to the GPU
is graphics, effects and their derivatives; physics will follow if and when rapier3d gains GPU
support.

This is a personal, experimental project — not a stable production tool. The documentation
reflects that: it explains **what is**, not what should be. Where something is missing or
broken, this book says so and links the issue.

## Audience

Two readers, with overlapping needs:

1. **Engine users** — anyone writing a game on top of Kóoch. Start with
   [Your First Project](./scripting/first-project.md), then
   [The Editor](./editor/overview.md).
2. **Engine contributors** — anyone touching `crates/*`. Start with
   [Crate Graph](./architecture/crate-graph.md) and the
   [Decisions Log](./reference/decisions-log.md).

## Status

Early development. The pieces work together end to end — window, ECS, scene serialisation,
meshlet renderer, sky, physics, editor — but the feature surface is narrow on purpose and
APIs break freely.

What works today:

- A 26-crate Rust workspace plus a facade, edition 2024 — see
  [Crate Graph](./architecture/crate-graph.md).
- GPU-driven meshlet rendering with a LOD chain, plus glTF mesh loading. A frame is a list of
  views, so the editor's viewport and the game's camera render from one stage.
- **Cook-Torrance lighting** driven by the light components, with clustered lights and
  shadows — virtual shadow pages by default, cascades, spot maps and point cubes behind them.
  See [Lighting](./architecture/lighting.md).
- Rigid-body physics on rapier3d: colliders, joints, collision events, sensors, materials,
  custom gravity fields that sum, and a floating character controller.
- Procedural skies and materials authored in the Shader Graph.
- In-game UI from `.kxml` / `.kcss` files, with video backgrounds — see [UI](./guide/ui.md).
- Scene serialisation (`.scene`, RON) driven by reflection, with more than one scene
  loadable at once.
- Input as data: an action is an asset, with composites and processors, editable in a panel.
- An editor: viewport, hierarchy, Inspector, Console, asset browser, drag-and-drop, dockable
  layout, undo/redo, project Hub, a Game panel beside the View panel, block modelling, and
  Play/Stop that snapshots and restores the authored world. Hovering a field shows its doc
  comment, units included.
- A project's own components and systems, written in Rust: **Rebuild & Run** compiles them and
  reloads them into the editor, keeping the open scenes.
- Builds from the editor: presets per platform, an encrypted asset pack, a profiling mode —
  see [Shipping a Game](./guide/shipping.md).

What does not work yet, stated plainly:

- **Compile errors are not shown as a list.** A failing build is read in the Console
  ([#158](https://github.com/lobinuxsoft/kooch/issues/158)).
- **Reflection is partial.** Nested structs and `Vec<T>` work; maps and enums carrying data
  do not ([#649](https://github.com/lobinuxsoft/kooch/issues/649)).
- **No authorable audio.** `kooch_audio` plays sound, but there is no `AudioSource`
  component yet ([#1645](https://github.com/lobinuxsoft/kooch/issues/1645)).
- **No global illumination**, which is why punctual light defaults are larger than physics
  says they should be ([#450](https://github.com/lobinuxsoft/kooch/issues/450)). The
  [Lighting](./architecture/lighting.md) page explains the trade rather than hiding it.

## Stack

| Layer | Crate / Library |
|---|---|
| GPU | `wgpu 30` (Vulkan / DX12 / Metal) |
| Windowing | `winit 0.30` |
| Math | `glam 0.33` |
| Physics | `rapier3d 0.36` |
| Audio | `kira 0.12` |
| Input | `gilrs 0.11` (gamepad), winit (keyboard/mouse) |
| UI layout | `taffy 0.15`, `vello 0.11` for vector shapes |
| Gameplay code | Plain Rust — native plugin via `kooch_plugin_api` |
| Editor UI | `egui 0.36` + `egui_dock 0.21` |
| Mesh | `gltf 1.4` |
| Serialisation | `serde 1` + `ron 0.11` |

## License

**All Rights Reserved.** Copyright (C) 2025-2026 Matías Galarza ("Lobinux", lobinuxsoft).

The repository is public so the work can be read and so the project can use branch protection
and Pages. That is not a licence to use it: see
[`LICENSE.md`](https://github.com/lobinuxsoft/kooch/blob/main/LICENSE.md).

## How to read this book

**User Guide** and **Scripting** are the public API surface. **Architecture** covers
internals. **Reference** holds the long-form material: the Decisions Log is a chronological
record of architectural choices, why they were made, and what was traded away.

Two files in the repository outrank this book when they disagree:
[`docs/MEMORY.md`](https://github.com/lobinuxsoft/kooch/blob/development/docs/MEMORY.md)
is canonical on **decisions**, and
[`docs/ROADMAP.md`](https://github.com/lobinuxsoft/kooch/blob/development/docs/ROADMAP.md)
is canonical on **order**.
