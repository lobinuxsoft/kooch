# OhMyGameEngine

A GPU-driven game engine written in Rust, featuring SDF-based rendering and compute-first architecture.

## Overview

OhMyGameEngine (OME) is an experimental game engine that leverages modern GPU capabilities for rendering, physics, and entity management. Instead of traditional rasterization, it uses **Signed Distance Fields (SDF)** and **ray marching** for rendering.

## Features

- **GPU-Driven ECS**: Entity Component System designed to run on the GPU
- **SDF Rendering**: Ray marching renderer using Signed Distance Fields
- **Multi-Gravity System**: Mario Galaxy-style gravity fields
- **Spatial Audio**: 3D audio with kira backend
- **Hot-Reload Scripting**: Rhai scripting integration
- **Integrated Editor**: Built-in editor overlay and standalone editor

## Architecture

```
oh_my_engine/
├── crates/
│   ├── ome_core         # App, Plugin system, GPU context (wgpu)
│   ├── ome_ecs          # GPU-driven Entity Component System
│   ├── ome_window       # Windowing (winit)
│   ├── ome_input        # Keyboard, mouse, gamepad (gilrs)
│   ├── ome_sdf          # SDF primitives and operations
│   ├── ome_lighting     # Point, spot, directional, area lights
│   ├── ome_render       # Ray marching renderer
│   ├── ome_physics      # GPU physics + SDF collision
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
