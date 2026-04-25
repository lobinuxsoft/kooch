# Introduction

**Oh My Engine** (OME) is a GPU-driven game engine written in Rust. Its primary
rendering path is **signed-distance-field ray-marching** rather than
rasterization, with a hybrid CPU/GPU entity-component-system (ECS) where
gameplay logic lives on the CPU and physics + render data live on the GPU.

This is a personal, experimental project — not a stable production tool. The
documentation reflects that reality: it explains *what is*, not *what should
be*. Decisions get recorded so future-me (and you) can reconstruct the
reasoning behind any line of code.

## Audience

Two distinct readers, with overlapping needs:

1. **Engine contributors** (anyone touching `crates/*`): need architecture
   maps, decision rationale, and the *why* behind module boundaries. Start
   with [Crate Graph](./architecture/crate-graph.md) and the
   [Decisions Log](./reference/decisions-log.md).

2. **Engine users** (anyone writing a game on top of OME): need a working
   project template, an explanation of the Bevy-style `App` / `Plugin` /
   `Stage` model, and component/scene reference. Start with
   [Getting Started](./guide/getting-started.md).

## Status

Early development. Core systems exist and work together end-to-end (window,
ECS, scene serialization, ray-march renderer, mesh renderer, sky renderer,
editor), but feature surface is intentionally narrow. There are no
backwards-compatibility guarantees yet — APIs break on every minor version.

What works today:

- 14-crate Rust workspace, edition 2024.
- SDF primitives + ray-march renderer (sphere tracing, adaptive epsilon,
  smooth blends).
- glTF mesh loading + rasterized mesh pass with depth-tested compositing
  against the SDF buffer.
- Procedural sky + volumetric clouds.
- Scene serialization (`.ome_scene` RON format) with reflection-driven
  components.
- Editor with viewport, hierarchy, inspector, drag-and-drop, dock layout
  persistence, undo/redo, project launcher, Play/Stop.
- Play action that re-runs the user's project crate with the current ECS
  state via `cargo run --manifest-path`.

## Stack

| Layer       | Crate / Library |
|-------------|-----------------|
| GPU         | `wgpu 29` (Vulkan / DX12 / Metal abstraction) |
| Windowing   | `winit 0.30` |
| Math        | `glam 0.29` |
| Audio       | `kira 0.9` |
| Input       | `gilrs 0.11` (gamepad), winit (keyboard/mouse) |
| Scripting   | `rhai 1.21` |
| Editor UI   | `egui 0.34` + `egui_dock 0.19` |
| Mesh        | `gltf 1.4` |
| Serial.     | `serde 1` + `ron 0.8` |

## License

Proprietary — All Rights Reserved. See repo `LICENSE` if present.

## How to read this book

The Architecture section covers internals. The User Guide covers the
public API surface. The Reference section is where long-form material lives:
the Decisions Log is a chronological record of architectural choices, why
they were made, and what tradeoffs were accepted.

If you only have time for two pages, read [Crate Graph](./architecture/crate-graph.md)
and [Getting Started](./guide/getting-started.md). Everything else can be
recovered by `git log` + this book.
