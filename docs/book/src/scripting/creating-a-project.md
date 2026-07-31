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

The engine dependency carries feature flags, and each one buys something specific:

```toml
kooch = { path = "…", features = [
    "editor",                 # the embedded editor, so `cargo run` opens it
    "physics",                # rigid bodies — without it, RigidBody is inert
    "gravity",                # gravity sources — without it, PointGravity pulls on nothing
    "remote",                 # `--remote`, so the standalone editor can drive this project
    "physics-debug-render",   # the solver's own account of itself, for the overlay
    "dynamic",                # the plugin API — without it, lib.rs does not compile
] }
```

`dynamic` is the one that is not optional in practice: leave it out and `lib.rs` fails to
build, because `kooch::kooch_plugin_api` is compiled out.

### `main.rs` — three ways to run

```rust
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--game") {
        // The game. Systems run.
        let mut app = App::new();
        app.add_plugins(DefaultPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: true });
        app.run();
    } else if args.iter().any(|a| a == "--remote") {
        // Headless authoring host for the standalone editor.
        // Systems register but start paused; the editor's Play flips them on.
        let mut app = App::new();
        app.add_plugins(RemoteHostPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: false });
        app.add_plugin(kooch::kooch_remote::RemotePlugin::new());
        app.run();
    } else {
        // The editor, embedded in your project.
        kooch::kooch_editor_core::run_editor_with(
            registrations::ProjectRegistrations { run_systems: false },
        );
    }
}
```

| Command | What you get |
|---|---|
| `cargo run` | The editor, with your components in it |
| `cargo run -- --game` | The game |
| `cargo run -- --remote` | A headless host for the standalone editor to drive |

`--remote` is headless on purpose: the editor draws that world in its own viewport, so a
window here would show the same scene twice.

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
impl kooch::kooch_plugin_api::OmePlugin for ProjectPlugin {
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
