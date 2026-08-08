# Getting Started

You need a recent Rust toolchain (edition 2024) and this repository cloned somewhere.

```bash
cargo run -p kooch_editor
```

That opens the Hub, where projects are created and opened. Everything else follows from
there:

- **[Your First Project](../scripting/first-project.md)** — the whole loop end to end: a
  component, a system, and Play. Start here.
- **[Creating a Project](../scripting/creating-a-project.md)** — what the scaffold generates
  and why each file exists.
- **[The Editor](../editor/overview.md)** — the panels, how Play works, and what is not built
  yet.

The rest of this page is the handful of things that are easy to trip over and do not belong
to any one of those.

## A project carries its own engine

Creating a project copies the engine's source into `<project>/engine/`,
and the generated `Cargo.toml` refers to it by a **relative** path:

```toml
kooch = { path = "engine", features = [...] }
```

Commit that directory. It is ~8 MB of text, and it is what makes the
project build on any machine, at the engine revision it was authored
against, with nothing installed beside it. `engine/target` is
gitignored; the source is not.

**It costs no build time.** A project always compiled the engine from
source — it just used to reach outside its own directory to find it.

⚠️ **Rust is still required** to build a project. Gameplay is native
Rust compiled into the game, so the toolchain is not optional the way it
is in an engine whose gameplay is a script.

### Developing the engine itself

When the editor runs out of the engine's own `target/`, project creation
skips the copy and points the manifest at the live clone instead —
otherwise every change to the engine would need a re-vendor before the
game could see it. The check is where the *executable* is, not where the
source is ([`engine_vendor::running_from_engine_build`]).

[`engine_vendor::running_from_engine_build`]: https://github.com/lobinuxsoft/kooch/blob/development/crates/kooch_editor_core/src/engine_vendor.rs

## Loading a scene

The boot scene is resolved in this order:

1. `SceneBootstrapPlugin::with_scene(path)`, if your `main.rs` sets one explicitly.
2. `--scene <path>` on the command line — absolute, or relative to the working directory.
3. `scenes/default.scene`, relative to the working directory.

So `cargo run -- --game` from the project root just works: the default path resolves because
the working directory is the project. A different level is
`cargo run -- --game --scene scenes/Level1.scene`.

## Component registration runs before the scene loads

`SceneBootstrapPlugin` loads at `Stage::First`, which runs **after** every `Stage::Startup`
system has completed on the first frame. Component registration is a `Startup` system, so the
registry is fully populated by the time the scene is deserialised.

Flip that order and you get `unknown component type: …` and a scene that does not load. The
generated `registrations.rs` already puts registration in `Startup`; this matters only if you
register something by hand.

## A scene with no camera renders black

The editor's own camera is filtered out of a saved scene, so a scene needs to spawn its own
`PerspectiveCamera`. The default scene template includes one. Build a scene from scratch
without one and you get the clear-to-black fallback.

This is deliberate, not a bug. Injecting the editor camera as a temporary play camera —
what Unity and Unreal do — is a possible future change, not current behaviour.

## Running without the editor

```bash
cargo run -- --game
```

`DefaultPlugins` is the group that makes this a game rather than a collection of crates:

| Plugin | Role |
|---|---|
| `CorePlugin` | `Time`, the `AppExit` event |
| `EcsPlugin` | Storage, `SceneManager`, built-in components, transform propagation |
| `WindowPlugin` | Winit window and GPU surface |
| `RenderPlugin` | The mesh and sky pipelines |
| `SceneBootstrapPlugin` | Loads the boot scene at startup |

Your project's own `main.rs` is what runs, so any plugin you add there is picked up.
