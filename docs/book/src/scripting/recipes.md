# Recipes

Code a game writes over and over, and the one call that replaces it. Each recipe is the pattern,
the helper, and the two side by side. Everything here is in `kooch::prelude` unless it says
otherwise.

The rule behind them: a pattern that turns up in three places becomes a helper, named for what it
does. When the pattern exists only because the engine is missing something, the fix goes in the
engine instead — so a few patterns below are still written by hand, and say why.

## One entity's component

**Before**

```rust
# use kooch::prelude::*;
# fn f(registry: &ComponentRegistry, entity: Entity) {
let transform = registry.get_cpu::<Transform>().and_then(|all| all.get(entity));
# let _ = transform;
# }
```

**After**

```rust
# use kooch::prelude::*;
# fn f(registry: &ComponentRegistry, entity: Entity) {
let transform = registry.component::<Transform>(entity);
# let _ = transform;
# }
```

For many entities, take the view once (`get_cpu`) and look each one up in it; for anything that
iterates, use a `Query`.

## The frame's time

Every system used to ask for `Time` and invent its own fallback for when it was missing — `0.0` in
some, `1/60` in others.

**Before**

```rust
# use kooch::prelude::*;
# fn f(resources: &Resources) {
let dt = resources.get::<Time>().map(|time| time.delta_secs()).unwrap_or(1.0 / 60.0);
# let _ = dt;
# }
```

**After**

```rust
# use kooch::prelude::*;
# fn f(resources: &Resources) {
let dt = Time::of(resources).delta_secs();
# let _ = dt;
# }
```

One answer everywhere, test harnesses included. Which clock to read is [Time](../guide/time.md).

## A button pressed this frame

An action read as "held" fires every frame the button is down. A jump wants the frame it went
down.

**Before** — remembering last frame's state per action:

```rust
# use kooch::prelude::*;
# struct Input { jump: Option<Guid>, was_down: bool }
# fn f(loaded: &LoadedActions, input: &mut Input) {
let down = loaded.pressed(input.jump);
let jumped = down && !input.was_down;
input.was_down = down;
# let _ = jumped;
# }
```

**After**

```rust
# use kooch::prelude::*;
# struct Input { jump: Option<Guid> }
# fn f(loaded: &LoadedActions, input: &Input) {
let jumped = loaded.just_pressed(input.jump);
# let _ = jumped;
# }
```

`just_released`, `axis` and `vector2` read the same way, all by the action's `Guid`. See
[Input](../guide/input.md).

## A UI button was clicked

A button's `on:click="menu.resume"` sends a `UiEvent` with that action.

**Before**

```rust
# use kooch::prelude::*;
# fn f(resources: &Resources) {
let clicked: Vec<Entity> = resources
    .get::<Events<UiEvent>>()
    .map(|events| {
        events
            .read()
            .filter(|event| event.action == "menu.resume")
            .map(|event| event.document)
            .collect()
    })
    .unwrap_or_default();
# let _ = clicked;
# }
```

**After**

```rust
# use kooch::prelude::*;
# fn f(resources: &Resources) {
let clicked = ui_actions(resources, "menu.resume");
# let _ = clicked;
# }
```

It returns the documents that sent it, once per click. Read `Events<UiEvent>` yourself only when
you need the node or the value — a slider's position, a list item's `data-index`.

## Writing text into the UI

**Before**

```rust
# use kooch::prelude::*;
# fn f(trees: &mut UiTrees, document: Entity, score: u32) {
if let Some(tree) = trees.get_mut(document)
    && let Some(node) = tree.query("score")
{
    tree.write_text(node, &score.to_string());
}
# }
```

**After**

```rust
# use kooch::prelude::*;
# fn f(trees: &mut UiTrees, document: Entity, score: u32) {
trees.write_text(document, "score", &score.to_string());
# }
```

A node that is not there is skipped, which is what a HUD wants while its document is still
loading. To build nodes from code — a list of items — `ui_instance` instances a `.kxml` piece; see
[UI files](../guide/ui_files.md).

## A warning from something that runs every frame

A system that fails the same way each frame floods the Console with sixty lines a second.

**After**

```rust
# use kooch::prelude::*;
# fn f(error: &str) {
warn_once!("items: {error}");
# }
```

Once per call site for the whole run. There was no "before" worth showing — every site rolled its
own `static` flag.

## Rumbling the pad being played

**After**

```rust
# use kooch::prelude::*;
# fn f(resources: &mut Resources, impact: f32) {
if let Some(input) = resources.get_mut::<Box<dyn InputBackend>>() {
    input.rumble_active(Rumble { strong: impact, weak: impact * 0.5, duration_ms: 120 });
}
# }
```

`rumble_active` picks the pad last used, so a game never has to track which of four is the
player's. A keyboard player, or a pad with no motors, gets nothing and no error.

## A component that might be there

**Before** — two queries, or a registry lookup inside the loop.

**After**

```rust
# use kooch::prelude::*;
#[system]
pub fn report(resources: &mut Resources) {
    Query::<(&Transform, Option<&Grounded>)>::new(resources).for_each(|(at, grounded)| {
        let standing = grounded.is_some_and(|g| g.standing);
#       let _ = (at, standing);
    });
}
```

`Option<&T>` (and `Option<&mut T>`) in a query matches entities with or without `T`.

## A ray that does not hit the one casting it

**After**

```rust
# use kooch::prelude::*;
# fn f(world: &PhysicsWorld, body: SolverBody, at: Vec3) {
let below = world.raycast_where(at, Vec3::NEG_Y, 1.0, world.without(body));
# let _ = below;
# }
```

Without the filter a ray from inside a body finds that body first. See
[Physics](../guide/physics.md#asking-the-world).

## Which way is up

**After**

```rust
# use kooch::prelude::*;
# fn f(resources: &Resources, at: Vec3) {
let up = gravity_up(resources, at);
# let _ = up;
# }
```

Away from the gravity that acts at that point — on a planet, the surface normal; with no field,
world up. Every system that asks this way agrees with the solver about where the floor is; one that
works it out differently ends up disagreeing. See [Gravity](../guide/gravity.md).

## Still written by hand

These patterns are known, counted, and waiting on the engine rather than on a helper:

- **Reading with a `Query`, then changing a resource.** A query borrows the resources, so a system
  collects what it found into a `Vec` and takes `&mut PhysicsWorld` after — every example in
  [Physics](../guide/physics.md) does. The fix is queries and resources as system parameters,
  [#1720](https://github.com/lobinuxsoft/kooch/issues/1720).
- **Lifting a resource out to use it beside the others** — `remove::<T>()`, work, `insert(t)` — and
  forgetting the `insert` on an early return. A scoped helper is planned in
  [#1724](https://github.com/lobinuxsoft/kooch/issues/1724).
- **Noticing that Play stopped.** Each system keeps its own "was playing" flag today; a `Stopped`
  event is planned in #1724 as well.
