# OhMyEngine

A hybrid CPU-GPU game engine written in Rust, featuring SDF-based rendering and compute-first architecture.

## Overview

OhMyEngine (OME) is an experimental game engine that leverages modern GPU capabilities for rendering and physics, while keeping gameplay logic on CPU for flexibility. Instead of traditional rasterization, it uses **Signed Distance Fields (SDF)** and **ray marching** for rendering.

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
oh_my_engine/
├── crates/
│   ├── ome_core         # App, Plugin system, Schedule, Resources
│   ├── ome_ecs          # Hybrid ECS (CPU gameplay + GPU physics data)
│   ├── ome_window       # Windowing (winit)
│   ├── ome_input        # Keyboard, mouse, gamepad (gilrs)
│   ├── ome_sdf          # SDF primitives and operations
│   ├── ome_lighting     # Point, spot, directional, area lights
│   ├── ome_render       # Ray marching renderer
│   ├── ome_physics      # GPU physics + batched queries
│   ├── ome_gravity      # Multi-gravity system
│   ├── ome_world        # Hierarchical coordinates, streaming
│   ├── ome_audio        # Spatial audio (kira)
│   ├── ome_scripting    # Rhai integration
│   ├── ome_editor_core  # Editor overlay
│   └── ome_editor       # Standalone editor (egui)
├── examples/
└── assets/
    └── shaders/
```

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
