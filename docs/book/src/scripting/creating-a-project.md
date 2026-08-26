# Creating a Project

<!-- toc -->

A project is an ordinary Cargo crate that depends on the engine. The editor scaffolds it, but
nothing about it is magic — you can read every generated file, and most of them you will
never touch.

## What the editor generates

```
MyGame/
├── Cargo.toml
├── assets/
├── scenes/
└── src/
    ├── main.rs            # generated, yours to edit
    ├── lib.rs             # generated, editor-managed
    └── registrations.rs   # generated, editor-managed — do not edit
```

### `Cargo.toml`

The one line worth understanding:

```toml
[lib]
crate-type = ["rlib", "dylib"]
```

**Two artefacts from one crate.** The `dylib` is what the standalone editor loads to learn
your component types without compiling them. The `rlib` beside it is what your binary links,
so the shipped game is an ordinary statically linked executable — no dynamic loading at
runtime.

**The project declares its own features, and the default one is the game.**

```toml
[features]
default = ["game"]
game = ["kooch/physics", "kooch/gravity", "kooch/camera", "kooch/audio"]
editor = [
    "game",
    "kooch/editor",
    "kooch/remote",
    "kooch/dynamic",
    "kooch/physics-debug-render",
]

[dependencies]
kooch = { path = "…" }
```

Each one buys something specific. `physics` gives you rigid bodies — without it a
`PhysicsBody` is an inert component and nothing ever falls. `gravity` is the same story one
level up: a `PointGravity` that pulls on nothing. `camera` is the third: a `VirtualCamera`
that moves no camera.

The `editor` three are what authoring needs and a game does not: `kooch/editor` is the
embedded editor, `kooch/remote` is the socket the standalone editor drives your project
over, `kooch/dynamic` is the plugin API that lets it list your components without compiling
them, and `physics-debug-render` compiles the solver walk the physics overlay draws.

### 🔴 Why the game is the default, and why it matters

A shipped build must contain the game and nothing else. Bundling the editor ships the
*engine's authoring surface* next to your game — the tooling that authored it, in the same
artefact — plus a file-dialog stack, an HTTP listener, and a reflected description of every
type you registered.

`editor` is opt-in and the authoring binary asks for it with `required-features`, so the
guarantee belongs to the **build**, not to a `cfg` somebody has to get right. Check it
yourself:

```sh
cargo tree -e normal | grep kooch_editor_core          # nothing
cargo tree -e normal --features editor | grep kooch_editor_core   # there it is
```

Not "the linker drops it" — cargo never compiles it.

⚠️ **Reflection stays in a game build**, and that is not an oversight: a `.scene` is
deserialised by type name, so the game needs the registry to load its own scenes. What
leaves is the editor, the remote server and the plugin API.

### `main.rs` — the game, and nothing else

```rust
use PROJECT_CRATE::registrations;

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);
    app.add_plugin(registrations::ProjectRegistrations { run_systems: true });
    app.run();
}
```

No flags and no modes. Authoring lives in `src/editor.rs`, which is a second `[[bin]]`
gated behind the `editor` feature, so this file cannot reach it.

| Command | What you get |
|---|---|
| `cargo run` | Your game |
| `cargo run --features editor --bin <crate>_editor` | The editor, with your components |
| `cargo run --features editor --bin <crate>_editor -- --remote` | A headless host for the standalone editor to drive |

`--remote` is headless on purpose: the editor draws that world in its own viewport, so a
window here would show the same scene twice.

The editor passes `--features editor` on every build it runs for you, so none of this is
something to remember while authoring — it matters the day you ship.

⚠️ **Older projects are migrated when they open.** One exception: a `main.rs` you edited is
left exactly as it is, with a warning, because a migration that silently deleted a line of
your gameplay would be worse than one that did nothing. Move your setup into the plain
`App::new()` form above and the release build is the game only.

### `registrations.rs` — do not edit

The editor regenerates this file whenever you create or register a script. It scans `src/`
for two patterns and wires up what it finds:

- `impl Component for X` → a component
- `pub fn f(_: &mut Resources)` → a system

Detection is line-based rather than a full parse — enough for the generated templates and for
typical hand-written code, but it does mean an unusual formatting of those signatures can go
unnoticed. If a script you wrote does not show up, that is the first thing to check.

### `lib.rs` — the editor's entry point into your project

Also generated. It exports one plugin whose only job is to describe your components to a
standalone editor that loaded this `dylib`:

```rust
impl kooch::kooch_plugin_api::KoochPlugin for ProjectPlugin {
    fn name(&self) -> &str { "my_game" }
    fn build(&mut self, engine: &mut dyn kooch::kooch_plugin_api::Engine) {
        registrations::declare_components(engine);
    }
}

kooch::kooch_plugin_api::export_plugin!(ProjectPlugin);
```

## Opening an older project

Projects made with earlier versions of the editor are migrated on open: the `dylib` crate
type, the `dynamic` feature, and the `registrations` wiring are all added if missing. You do
not have to do anything, but the first build afterwards will be a full one.

## The compiler has to match

The `dylib` boundary carries Rust types directly rather than going through a C interface —
that is what makes the API pleasant. The price is that **the project and the engine must be
built by the same `rustc`**. A mismatch is refused with a clear message (the engine records
`rustc -V -v` at build time) rather than crashing, but it is refused.

In practice this is invisible, because the editor builds both. It becomes visible if you
update your toolchain and rebuild only one side.
