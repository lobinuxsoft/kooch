# Kooch

A hybrid CPU-GPU game engine written in Rust, featuring SDF-based rendering and compute-first architecture.

## Documentation

The book lives in [`docs/book/`](docs/book/src/SUMMARY.md). Build it with
[`mdbook`](https://github.com/rust-lang/mdBook) and the `mdbook-mermaid` /
`mdbook-toc` preprocessors:

```bash
cargo install mdbook mdbook-mermaid mdbook-toc
mdbook serve docs/book/
```

Start with [Introduction](docs/book/src/introduction.md), then either
[Crate Graph](docs/book/src/architecture/crate-graph.md) (for engine
contributors) or [Getting Started](docs/book/src/guide/getting-started.md)
(for engine users).

## Overview

Kooch (OME) is an experimental game engine that leverages modern GPU capabilities for rendering and physics, while keeping gameplay logic on CPU for flexibility. Instead of traditional rasterization, it uses **Signed Distance Fields (SDF)** and **ray marching** for rendering.

## Features

- **Hybrid ECS**: CPU handles gameplay logic (quests, inventory, AI), GPU handles physics and rendering
- **SDF Rendering**: Ray marching renderer using Signed Distance Fields
- **Multi-Gravity System**: Mario Galaxy-style gravity fields
- **Batched Physics Queries**: Raycasts/overlaps queued on CPU, executed in batch on GPU
- **Spatial Audio**: 3D audio with kira backend
- **Hot-Reload Scripting**: Rhai scripting integration
- **Integrated Editor**: Built-in editor overlay and standalone editor

## Hybrid Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         CPU SIDE                                │
├─────────────────────────────────────────────────────────────────┤
│  • Entity management (spawn/despawn, generational IDs)          │
│  • Gameplay logic (quests, inventory, dialogs, AI)              │
│  • Scripting (Rhai)                                             │
│  • Input processing                                             │
│  • Audio triggers                                               │
└──────────────────────────┬──────────────────────────────────────┘
                           │ Sync once per frame
                           ▼
┌─────────────────────────────────────────────────────────────────┐
│                         GPU SIDE                                │
├─────────────────────────────────────────────────────────────────┤
│  • Physics simulation (velocity, collisions, gravity)           │
│  • Batched physics queries (raycast, overlap)                   │
│  • Particle systems                                             │
│  • Transform hierarchy                                          │
│  • Ray marching rendering                                       │
└─────────────────────────────────────────────────────────────────┘
```

## Architecture

```
kooch/                  # Top-level facade crate (`kooch::*`)
├── src/                       # DefaultPlugins, SceneBootstrapPlugin, prelude
├── crates/
│   ├── kooch_core               # App, Plugin system, Schedule, Resources, GPU
│   ├── kooch_ecs                # Hybrid ECS (CPU gameplay + GPU data, scenes)
│   ├── kooch_ecs_macros         # #[derive(Reflect)], #[derive(Component)]
│   ├── kooch_plugin_api         # Stable ABI for dynamic plugins
│   ├── kooch_window             # Windowing (winit + GPU surface)
│   ├── kooch_input              # Keyboard, mouse, gamepad (gilrs)
│   ├── kooch_sdf                # SDF primitive math
│   ├── kooch_lighting           # Light components
│   ├── kooch_render             # Sky + raymarch + mesh pipelines
│   ├── kooch_physics            # GPU physics + batched queries
│   ├── kooch_gravity            # Multi-gravity system
│   ├── kooch_world              # Hierarchical coordinates, streaming
│   ├── kooch_audio              # Spatial audio (kira)
│   ├── kooch_scripting          # Rhai integration
│   ├── kooch_editor_core        # Editor as a library
│   └── kooch_editor             # Editor binary (egui)
├── docs/
│   ├── book/                  # mdBook source — see Documentation above
│   └── research/              # wgpu capabilities audit, etc.
├── examples/
└── assets/
    └── shaders/
```

See [Crate Graph](docs/book/src/architecture/crate-graph.md) for the
dependency layering and rationale.

## Tech Stack

| Category | Technology |
|----------|------------|
| Language | Rust (Edition 2024) |
| GPU API | wgpu |
| Windowing | winit |
| Audio | kira |
| Input | gilrs |
| Scripting | rhai |
| Editor UI | egui / eframe |

## Status

**Early Development** - Not ready for production use.

## License

All Rights Reserved. See [LICENSE.md](LICENSE.md) for details.
