# The Editor

<!-- toc -->

The editor is where a project is authored: entities are spawned, components are attached and
tuned, and the result is played back without leaving the window.

It is one program that runs in two arrangements, and the difference matters more than it
looks.

## Two arrangements, one editor

**Embedded** — the project's own binary opens the editor:

```bash
cargo run          # inside your project directory
```

The editor runs *inside* the project, so the project's components are simply linked in. This
is the simplest arrangement and needs no socket, no second process, and nothing to go wrong
between them.

**Standalone** — the editor opens, and you point it at a project:

```bash
cargo run -p ome_editor      # from the engine repo
# then: Open Project
```

Here the editor is a separate program that does not have your project's types compiled in. It
gets them two ways at once:

- It **loads the project's `dylib`** to learn what components exist and what fields they have,
  which is what fills the Add Component menu and the Inspector.
- It **launches the project** as a headless host (`--remote`) and drives it over a local
  socket. That host is where the world actually lives, so a system you wrote runs in *its*
  process while you watch the result in the editor's viewport.

> **Why the split exists.** Rust has no stable ABI, so a pre-built editor cannot simply link
> a project compiled separately. The `dylib` gets around that by requiring the same compiler
> for both — fine for code the editor itself built. The remote host covers the rest: running
> your systems against a live world. See [`docs/MEMORY.md`](https://github.com/lobinuxsoft/oh_my_engine/blob/development/docs/MEMORY.md)
> for the full reasoning.

The Hub — the window you get from `cargo run -p ome_editor` — is where projects are created
and opened.

![The Hub](../images/hub.png)

## The panels

| Panel | What it is for |
|---|---|
| **View** | The 3D viewport. Selection, transform gizmos, the physics debug overlay. |
| **World** | The entity hierarchy of every loaded scene. Selecting here selects in View. |
| **Inspector** | The selected entity's components and their fields. Where authoring happens. |
| **Components** | Every component type the engine and your project registered. |
| **Archetypes** | Which combinations of components actually exist, and how many entities are in each. A debugging view of how the ECS stored your scene. |
| **Asset Browser** | The project's assets and the engine's, as two roots. |
| **Console** | Structured logs from the editor *and* the launched project, filterable. Text is selectable and copyable. |
| **Performance** | Frame timings, and per-stage counters where they exist. |

## Play

Play does **not** rebuild anything and does **not** open a second window.

Pressing Play snapshots the authored world, flips the `Playing` gate so gameplay systems
start running, and simulates in the editor's own viewport. Stop lowers the gate and restores
the snapshot, so the world goes back exactly as authored — you do not lose your scene by
testing it.

```rust
// ome_remote::handlers::set_playing, in essence
if playing {
    resources.insert(PlaySnapshot(WorldSnapshot::capture(resources)));
    Playing::set(resources, true);
}
// Stop: the gate goes down *before* the restore, so no system
// observes a half-rebuilt world.
Playing::set(resources, false);
if let Some(snapshot) = resources.remove::<PlaySnapshot>() {
    snapshot.0.restore(resources);
}
```

This is why your project's systems register with `run_systems: false` while authoring: they
are registered either way and skipped per frame, so Play can flip them on live rather than
recompiling.

> **Known rough edge.** A locally-opened project's Play button still has an older path that
> shells out to `cargo run -- --game`, which builds the project and opens a second window —
> minutes of nothing, and no snapshot. Tracked in
> [#633](https://github.com/lobinuxsoft/oh_my_engine/issues/633).

## What the editor does not do yet

Honest list, so nothing below is mistaken for a bug in your setup:

- **No build button.** Changing Rust code means running `cargo build` yourself
  ([#158](https://github.com/lobinuxsoft/oh_my_engine/issues/158)).
- **No reload.** The project's `dylib` loads once, when the project opens. Seeing a code
  change means reopening the editor
  ([#648](https://github.com/lobinuxsoft/oh_my_engine/issues/648)).
- **No New Scene.** Scenes have to exist on disk already
  ([#619](https://github.com/lobinuxsoft/oh_my_engine/issues/619)).
- **The editor redraws while idle**, pinning a core with nothing happening
  ([#656](https://github.com/lobinuxsoft/oh_my_engine/issues/656)).
