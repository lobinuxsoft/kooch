# Writing a System

<!-- toc -->

A system is a function. That is the entire type:

```rust
pub fn my_system(resources: &mut Resources) { … }
```

No trait to implement, no macro, no parameter-injection magic. `Resources` is the world, and a
system does whatever it wants with it.

## The smallest one that works

```rust
use kooch::prelude::*;

/// Ticks every entity's regeneration.
pub fn regenerate_health(resources: &mut Resources) {
    let _ = resources;
}
```

The editor's New System command drops this scaffold in `src/` and regenerates
`registrations.rs`, which picks it up by its signature.

## Reading and writing components

Components are reached through `Query`, which is constructed from `Resources` and borrows what
it names:

```rust
use kooch::prelude::*;
use kooch::kooch_ecs::query::Query;

use crate::health::Health;

pub fn regenerate_health(resources: &mut Resources) {
    let dt = resources
        .get::<Time>()
        .map(|t| t.delta_secs())
        .unwrap_or(1.0 / 60.0);

    let query = Query::<&mut Health>::new(resources);
    query.for_each(|health| {
        health.current = (health.current + 5.0 * dt).min(health.max);
    });
}
```

A few shapes worth knowing:

```rust
Query::<&Health>::new(resources)                 // read one component
Query::<&mut Health>::new(resources)             // write one component
Query::<(&Transform, &mut Health)>::new(res)     // entities that have both
```

and on the query itself:

| Call | Use |
|---|---|
| `.iter()` | An iterator, when you want to `collect`, `sum`, `filter` |
| `.for_each(\|item\| …)` | The common case |
| `.for_each_entity(\|entity, item\| …)` | When you need the entity id too, e.g. to look up an optional component |
| `.get(entity)` | One specific entity, `None` if it does not match |
| `.is_empty()` | Cheap early-out |

**Conflicting borrows panic rather than corrupt.** Holding `&mut Health` in two live queries
at once is caught by the access tracker. Scope a query with a block when you need to release
it before building the next one.

## Stages

A system is registered into a stage, and stages run in a fixed order every frame:

| Stage | For |
|---|---|
| `Startup` | Once, at startup |
| `First` | Beginning of frame |
| `Input` | Input processing |
| `PreUpdate` | Preparation |
| **`Update`** | **Your game logic — the default choice** |
| `PostUpdate` | Cleanup after update |
| `GpuSync`, `Gpu` | GPU sync and submission |
| `Physics`, `PostPhysics` | Fixed timestep |
| `PreRender`, `Render` | Rendering |

If you do not have a reason, `Update` is the reason.

Physics runs on a **fixed** timestep, so a system in `Physics` or `PostPhysics` should use
`Time::fixed_delta_secs()` rather than `delta_secs()`. Using the wrong one is a bug that only
shows up when the frame rate changes.

## The `Playing` gate

Your systems are registered whether or not the game is running, and skipped per frame while it
is not. That is what lets the editor's Play button start gameplay without a rebuild.

`registrations.rs` does it by wrapping each of your systems:

```rust
app.insert_resource(Playing(self.run_systems));            // false while authoring
app.add_system(Stage::Update, run_if_playing(my_system));  // skipped while the gate is down
```

What it means in practice: **a system must not assume it runs every frame from startup.** It
may start running at any moment, against a world somebody has been editing by hand — and stop
again when Stop restores the authored snapshot underneath it.

## Spawning and despawning

Structural changes go through `Commands`, not through `Query`, because adding a component
moves an entity between archetypes and that cannot happen while a query is iterating it.

`Commands::spawn` needs `&mut Resources` itself, so it cannot be borrowed *out* of resources
while it is used. Take it out, use it, put it back:

```rust
use kooch::kooch_ecs::commands::Commands;

pub fn spawn_a_pickup(resources: &mut Resources) {
    let Some(mut commands) = resources.remove::<Commands>() else { return };

    let entity = commands
        .spawn(resources)
        .insert(Health { current: 100.0, max: 100.0 })
        .id();

    commands.apply(resources);
    resources.insert(commands);
    let _ = entity;
}
```

Two things that catch people:

- **The id is allocated immediately; the components are not.** `spawn` returns a valid
  `Entity` straight away, but the inserts are queued until `apply`. Querying that entity
  before `apply` finds nothing on it.
- **Put `Commands` back.** `remove` takes it out of `Resources`; anything running later that
  expects it there will not find it.

To despawn:

```rust
commands.entity(target).despawn();
```

## Registration

You do not write it. The editor finds `pub fn regenerate_health(_: &mut Resources)` and
regenerates `registrations.rs`:

```rust
app.add_system(Stage::Update, run_if_playing(health::regenerate_health));
```

**`Update` is the only stage it picks**, and it is not configurable from the editor. To run
somewhere else, register the system by hand in your own plugin rather than fighting the
generated file — `registrations.rs` is overwritten without warning.
