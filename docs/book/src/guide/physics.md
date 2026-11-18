# Physics

A body is two components: `PhysicsBody` says whether the solver moves it, `Collider` says what
shape it has. Put both on an entity and press Play. There is no "create body" call, no handle to
keep — the engine builds the solver's body from the components and rebuilds it when you edit them.

This page is about bodies, pushing them, asking the world questions and hearing what touched what.
The shapes themselves — which one to pick, baking a hull from a mesh, friction and layers — are
[Colliders](colliders.md).

Physics needs the `physics` feature, which a project created by the editor already has.

## The body

| Field | What it does |
|---|---|
| `kind` | `Dynamic` (the solver moves it), `Kinematic` (you move it, it pushes others), `Static` (nothing moves it) |
| `mass` | Kilograms — the **whole** mass. Colliders add none |
| `density` | Only for the **Calculate mass** button. The simulation never reads it |
| `center_of_mass` | Off by default (the collider's centre). A car wants it low |
| `linear_damping`, `angular_damping` | Motion lost with nothing touching. Zero keeps it forever |
| `gravity_scale` | 1 normal, 0 weightless, negative a balloon |

Static and kinematic bodies hide the fields they ignore; the values stay, so switching back to
Dynamic restores them.

**Moving a kinematic body is moving its `Transform`.** The solver follows it and pushes what is in
the way. Moving a *dynamic* body's `Transform` also works, but it is a teleport: the body keeps no
velocity from it. To move a dynamic body, push it.

### Where the world comes from

The solver runs only while the game plays. The bodies are rebuilt from the components on every
Play, so nothing the solver does reaches a saved scene — Stop puts every body back where you
authored it.

The step is fixed: `Time` accumulates the frame and `Physics` runs once per whole step, so a
body falls the same at 30 fps and at 144. Gravity is −9.81 on Y unless the app says otherwise
(`PhysicsPlugin::new().with_gravity(..)`) or a scene adds a [gravity source](gravity.md).

## Pushing a body

A system reaches a body through two things in the prelude: `SolverBody`, the component the engine
adds to every simulated entity, and `PhysicsWorld`, the resource that owns them.

```rust
# use kooch::prelude::*;
# #[derive(Default, Reflect)]
# pub struct Thruster { pub force: f32 }
# impl Component for Thruster {}
/// Pushes every thruster forward along its own facing.
#[system]
pub fn thrust(resources: &mut Resources) {
    // Read first, then take the world: a `Query` borrows the resources, and so does `get_mut`.
    let mut pushes = Vec::new();
    Query::<(&Thruster, &SolverBody, &Transform)>::new(resources).for_each(|(thruster, body, t)| {
        pushes.push((*body, t.rotation * Vec3::NEG_Z * thruster.force));
    });
    let dt = Time::of(resources).delta_secs();
    let Some(world) = resources.get_mut::<PhysicsWorld>() else {
        return;
    };
    for (body, force) in pushes {
        world.apply_impulse(body, force * dt);
    }
}
```

The world's methods take a `SolverBody` and do nothing for a body that is gone — ordinary
after Stop, not an error.

| Method | Use it for |
|---|---|
| `apply_impulse(body, n_s)` | A push, in newton-seconds. Always wakes the body |
| `apply_torque_impulse(body, ..)` | A spin. A rolling ball wants this: friction carries it |
| `linear_velocity`, `angular_velocity` | Reading how it moves |
| `set_linear_velocity`, `set_angular_velocity` | Overriding it. Discards this step's bounce and slope |
| `set_rotation(body, q)` | Facing a character. Pair it with `set_angular_velocity(Vec3::ZERO)` |
| `teleport(body, position)` | A respawn or a portal: moves it and stops it. Writing a dynamic body's `Transform` in Play does nothing |
| `transform(body)` | Where the solver has it, before the `Transform` catches up |
| `mass(body)`, `is_sleeping(body)` | What it says |

**Prefer the impulse.** `set_linear_velocity` is the one that makes a body ignore the floor it
just landed on, because the solver's answer for this step is thrown away.

## Asking the world

| Query | Answers |
|---|---|
| `raycast(origin, dir, max)` | The first thing along a ray |
| `raycast_where(.., filter)` | The same, through a `QueryFilter` |
| `sweep(shape, dir, max, filter)` | Where a shape moving that way first touches — a move test |
| `overlaps(shape, filter, out)` | Everything a shape overlaps where it stands |
| `project_point(p, max, filter)` | The nearest surface, and whether `p` is inside it |

A body asking about its surroundings has to **leave itself out**, or a ray cast down from a ball's
centre finds the ball. `world.without(body)` is that filter:

```rust
# use kooch::prelude::*;
/// Whether there is ground within `reach` below `at`, ignoring the body asking.
fn on_ground(world: &PhysicsWorld, body: SolverBody, at: Vec3, reach: f32) -> bool {
    world
        .raycast_where(at, Vec3::NEG_Y, reach, world.without(body))
        .is_some()
}
```

A hit names the solver's handle, not an entity; `world.entity_of(hit.body)` turns it into one.
`t` is in lengths of the direction you passed, so pass it normalised and `t` is metres.

## Hearing a collision

Contacts are **opt-in per collider**: tick `collision_events` for touch start and stop, and
`contact_force_events` for "hit harder than `contact_force_threshold`". A scene pays only for the
contacts something listens to. **A sensor (`is_trigger`) needs `collision_events` too** — ticking
the trigger alone makes it overlap silently. A sensor does hear everything that moves, kinematic
character controllers included.

The events come out as four buffers:

| Event | Sent when |
|---|---|
| `CollisionStarted { a, b, sensor }` | Two entities started touching, once |
| `CollisionStopped { a, b, sensor }` | They stopped |
| `ContactForce { a, b, total_force_magnitude, max_force_magnitude }` | A contact passed its threshold |
| `JointBroke { joint, a, b, impulse }` | A breakable joint tore off |

`a` and `b` come in no particular order. Ask about both:

```rust
# use kooch::prelude::*;
# #[derive(Default, Reflect)]
# pub struct Goal;
# impl Component for Goal {}
/// Logs whoever reaches a goal.
#[system]
pub fn reach_goal(resources: &mut Resources) {
    let Some(events) = resources.get::<Events<CollisionStarted>>() else {
        return;
    };
    let goals = Query::<&Goal>::new(resources);
    for hit in events.read().filter(|hit| hit.sensor) {
        let arrived = match (goals.get(hit.a), goals.get(hit.b)) {
            (Some(_), None) => hit.b,
            (None, Some(_)) => hit.a,
            _ => continue,
        };
        info!("{arrived:?} reached the goal");
    }
}
```

⚠️ **An event arrives the frame after the contact.** The buffers are double-buffered so a reader
in any stage sees every event once; the price is a frame. Nothing gameplay-visible at 60 fps, but
do not expect the bodies to still be where they met.

"Started" is said **once**. To know who is inside a sensor *now*, don't count starts and stops
yourself — the `SensorOccupancy` resource (`kooch::kooch_ecs::SensorOccupancy`) already does, and also says how far in each body is
(`depth_in(sensor)`), which is what a region that fades in wants.

## Joints

A `Joint` is a component on its **own** entity that names two bodies, `body_a` and `body_b`. It is
not on either body, so one body can be in several joints — a ragdoll's pelvis is in four.

| Kind | Leaves free |
|---|---|
| Fixed | Nothing — welds them |
| Revolute | Rotation about `axis` — a door |
| Prismatic | Sliding along `axis` — a piston |
| Spherical | All rotation — a shoulder |
| Rope | Anything under `max_length`, rigid at it |
| Spring | Everything, pulled back towards `rest_length` |
| Pin slot | Sliding along `axis` and spinning about it |
| Generic | Whatever `locked_axes` does not lock |

Anchors and `axis` are in `body_a`'s local space: the hinge belongs to the door frame, not the
world, so moving the frame moves the hinge.

- **Limits** (`limits_enabled`, `limit_min`, `limit_max`) bound the free axis — radians for a
  rotation, metres for a slide.
- **A motor** drives it towards `motor_target_position` and `motor_target_velocity`. The
  acceleration model reaches the target at the same rate whatever the mass; the force model makes
  a heavy door slower.
- **`breakable`** tears the joint off above `break_impulse` and sends `JointBroke`.
- **`articulated`** solves the chain as one body: no drift, more cost, and no loops.
- **`contacts_enabled`** is off: a door overlaps its frame, and the two would fight.

A joint with either body unset does nothing; it is inert until both are named.
