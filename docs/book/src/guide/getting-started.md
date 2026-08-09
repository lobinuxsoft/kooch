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

## One engine per machine, shared by every project

The editor materialises the engine once per version in

```text
~/.local/share/kooch/<version>/engine
```

and every project's `Cargo.toml` points at it:

```toml
kooch = { path = "/home/you/.local/share/kooch/0.1.0/engine", features = [...] }
```

**Nothing is copied into the project.** Two projects on the same engine
version share one directory; two versions coexist, so a project pinned
to an older engine keeps building after the editor updates.

⚠️ That path is absolute and `$HOME` differs per user, so a project that
changes machines names a directory that is not there. **The editor owns
that line** — it owns the directory it names — and rewrites it when a
project opens. Nothing to do by hand.

`KOOCH_ENGINE_HOME` overrides the base, for CI and for portable installs
that must not write to the user's data directory.

**It costs no build time.** A project always compiled the engine from
source; this only changes where the source is.

### When that directory is replaced

Every materialised engine records which source tree it came from, in a
`.kooch-engine-stamp` beside it — the version, plus a digest of every
file. An editor compares its own source against that stamp and replaces
the directory when they differ, leaving one copy behind, never two.

🔴 **Without it, a new editor never updated the engine.** The directory
is named after the engine version, that version is `0.1.0` for every
development build, and the old check only asked whether `Cargo.toml`,
`crates` and `src` existed — which is true of every copy of the engine
ever made. So a freshly installed editor found the directory, called it
current, and every project on the machine went on compiling against
weeks-old source with nothing said.

⚠️ **The build right after a replacement is a full rebuild**, since every
engine source file is now newer than the project's `target/`. The editor
logs it for that reason.

A version this editor does **not** ship is never touched: that directory
is what a pinned project builds against, and differing from the source in
hand is the reason it exists rather than a reason to overwrite it.

### Checking a copy that went wrong

The comparison above catches a *stale* engine, not a *damaged* one:
deleting a file from a copy does not change what the copy claims to be.

```sh
KOOCH_VERIFY_ENGINE=1 kooch_editor
```

re-reads the whole tree, compares it against its own stamp, and re-copies
when they differ. Off by default because it reads 8 MB every time a
project opens.

⚠️ **Rust is still required** to build a project. Gameplay is native Rust
compiled into the game, so the toolchain is not optional the way it is in
an engine whose gameplay is a script.

### Why the source is on disk at all

Because Rust has no stable ABI. A precompiled `rlib` links only against
the exact compiler and the exact dependency versions that built it, and
cargo does not model binary dependencies — which is why no Rust engine
ships binaries, Bevy included. The only route to "binary, no source" is
an `extern "C"` API in the shape of Godot's GDExtension, and it costs the
typed ECS.

So the engine's source is protected the way Unreal protects theirs: **by
licence, not by hiding it.**

### The licence is not optional

`LICENSE.md` is vendored with the engine, and the facade compiles it in:

```rust
pub const LICENSE: &str = include_str!("../LICENSE.md");
```

A game links the engine as an `rlib`, so **that text is inside every
shipped executable**. It is not a file someone has to remember to copy;
removing it means not building.

### Packaging the editor

```sh
cargo build --release -p kooch_editor
cargo run --release --features editor --example package_editor -- dist/
```

```text
dist/
  kooch_editor      the binary
  engine/           7.7 MB — the source it materialises for projects
    .kooch-engine-stamp   which tree this is, so an install can tell
                          whether it is newer than what is on the machine
  assets/           what the editor itself renders with
```

`engine_vendor::vendor_source` looks in three places, in order:
`KOOCH_ENGINE_SOURCE`, `engine/` next to the executable, and the engine
root — which only resolves when running from the engine's own tree.

⚠️ `package_editor` **refuses a binary older than the source**. It once
shipped an editor built before this feature existed, and the AppImage
made from it wrote its own mount point into a project — a directory that
stops existing when the app closes.

⚠️ **It packages for the platform it runs on.** An editor for Windows
means running it on Windows, the same conclusion Bevy's release workflow
reaches: `metis` is vendored C, which makes cross-compiling more than a
target flag.

### Developing the engine itself

When the editor runs out of the engine's own `target/`, project creation
points the manifest at the live clone and materialises nothing —
otherwise every engine change would need a re-materialise before the game
could see it. The check is where the *executable* is, not where the
source is.

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
