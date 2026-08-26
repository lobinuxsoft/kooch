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
| `Startup` | Once, at launch. Load, allocate, seed. |
| `First` | The very top of the frame. |
| `Input` | Reading devices into intent. |
| `PreUpdate` | Preparing what `Update` will need. |
| **`Update`** | **Your game logic — the default choice** |
| `PostUpdate` | After gameplay, and where `Transform` becomes `GlobalTransform`. |
| `GpuSync` | Handing this frame's data to the GPU. |
| `Gpu` | Compute submitted with the frame's encoder. |
| `Physics` | Fixed timestep. May run several times a frame, or none. |
| `PostPhysics` | Same timestep, after the solver. |
| `PreRender` | Last chance before drawing. |
| `Render` | Drawing. |
| `PostRender` | After drawing. |
| `Last` | The very end of the frame. |

If you do not have a reason, `Update` is the reason.

🔴 **`PostUpdate` is the one that bites.** It is where a local `Transform` is resolved into the
`GlobalTransform` that meshes, lights and cameras actually read. Write a transform *before* it
and the change lands this frame. Write it *after* — `PostUpdate`, `Gpu`, anywhere later — and
everything downstream renders **one frame behind, forever**, with no error and no log line. The
symptom is shadows or child objects that lag when the camera moves, which is not a bug anybody
traces back to a stage.

Physics runs on a **fixed** timestep, so a system in `Physics` or `PostPhysics` should use
`Time::fixed_delta_secs()` rather than `delta_secs()`. Using the wrong one is a bug that only
shows up when the frame rate changes. It may also run **several times in one frame, or none at
all**, so nothing that must happen once per frame belongs there.

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

You do not write it. The editor watches `src/`, finds
`pub fn regenerate_health(_: &mut Resources)`, and regenerates `registrations.rs`:

```rust
app.add_system(Stage::Update, run_if_playing(health::regenerate_health));
```

It watches by **polling**, so saving from any editor is enough — including one that is not
this one. There is no button to press.

### Saying where it goes

`Update` behind the `Playing` gate is what a system gets when it says nothing. `#[system(...)]`
says otherwise:

```rust
#[system]                     // Update, gated by Play — the default
#[system(PreUpdate)]          // a different stage, still gated
#[system(PostUpdate, always)] // and running while you author, too
```

`always` drops the `Playing` gate. Reach for it when the work has to happen in the editor as
well: a gizmo, an overlay, a streaming pump. It is a word rather than something inferred
because it is the one thing no amount of reading a function can tell you — a system that must
run while paused looks exactly like a gameplay one.

The attribute **expands to nothing**. It is read by the editor when it regenerates the file;
the compiler passes your function through untouched, and deleting the attribute never breaks a
build. What it does do is *validate*: a stage that is not one of the fourteen is a compile error
naming them, where a comment with the same typo would leave the system in `Update` forever and
never say so.

### Registered is not compiled

⚠️ The generated file names your system within a second of you saving. **The editor still runs
the last build of your project.** Those two disagree until you rebuild, and the toolbar's
Resync button pulses while they do.

This matters because the gap is invisible: the editor lists a project's components and systems
out of its compiled dylib, so something added ten seconds ago exists in `registrations.rs` and
in no binary anywhere. The symptom is *"I pressed Play and my system did not run"*.
