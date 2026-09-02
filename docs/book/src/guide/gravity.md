# Gravity

Most engines have one gravity: a vector, pointing down, the same everywhere.
Kóoch has that too — and four more shapes, because "down" on a planet is a
different direction for every body standing on it.

Gravity is **opt-in for the scene, automatic for the body**. Add no source
and the world keeps the vector it always had. Add one, and every dynamic
body inside its reach is pulled by it. There is no component to put on the
falling thing.

## The sources

| Component | The shape it is |
|---|---|
| `GlobalGravity` | Uniform, everywhere, no falloff. The world vector, authorable |
| `PointGravity` | A planet — pull towards a centre |
| `PlaneGravity` | A floor — unbounded sideways, bounded and one-sided along its normal |
| `AreaGravity` | A box you are **inside**, with its own down |
| `BoxGravity` | A solid you stand on the **outside** of |

Each is a component on an entity, and the entity's transform places it. A
planet parented to a moving ship brings its gravity along.

### The two boxes are not variants of each other

`AreaGravity` is a *region*: a corridor that runs up a wall, a room that
flips over. One direction throughout, acting on whatever is inside it.

`BoxGravity` is a *solid*: a cube planet. You are outside it, and the pull
differs at every point around it — each face along its own normal, the edges
and corners turning continuously between them.

Same primitive, opposite job. The edges need no special case because the
direction is the gradient of the box's distance function: gravity that
follows a surface *is* that gradient.

### Authoring a planet

```rust
commands.spawn(&mut resources)
    .insert(Transform::from_position(Vec3::ZERO))
    .insert(PointGravity {
        // What a body feels standing on the surface.
        strength: 9.81,
        // Where the surface is. Quoting the strength at a distance is why
        // this is authorable — `G·M` is not a number anyone can picture.
        radius: 50.0,
        // Past this the source contributes nothing. Zero is unlimited.
        range: 500.0,
        inverse_square: true,
    });
```

`inverse_square: false` gives a field of constant strength inside `range`.
That is not physical, and it is often what a game wants: a small planet you
can walk on without the pull changing under your feet.

## Fields add

Overlapping sources sum. Two planets pull along the vector sum, and a body
travelling between them transitions smoothly with nobody choosing a blending
weight — superposition *is* the blend.

This is also why rapier's own gravity switches off the moment a scene has
any source. A planet pulling towards its centre plus a world vector pulling
down is a diagonal, and the author placed one planet. For a uniform field
alongside the others, add a `GlobalGravity`.

## When adding is the wrong answer

Sometimes a zone must *replace*: "inside this room down is `-X`, ignore the
planet". Summed, the room fights the planet and the result is a slant nobody
authored.

Add a `GravityPriority` to the room:

```rust
commands.spawn(&mut resources)
    .insert(Transform::from_position(room_centre))
    .insert(AreaGravity { direction: Vec3::X, ..Default::default() })
    .insert(GravityPriority { level: 1 });
```

A higher level suppresses every lower one **in proportion to how strongly it
reaches each point**. At the room's centre the planet is gone; across the
room's `falloff` band it comes back gradually. That is what keeps a body
from snapping direction as it walks out of the door.

So give an overriding zone a soft edge. `AreaGravity`, `BoxGravity` and
`PlaneGravity` have a `falloff` for exactly this. `PointGravity` claims
everything inside its `range` and nothing outside, so overriding with one is
a hard edge — and `GlobalGravity` reaches everywhere at full strength, so
raising *it* switches off the rest of the scene entirely.

Sources at the same level sum, as they always did. A source with no
`GravityPriority` sits at level 0, so adding the component to one entity
changes nothing about the others.

## Asking which way is down

```rust
// The summed field. What the solver applies.
let pull = gravity_at(&resources, position);

// Up: away from that pull. World up where nothing reaches.
let up = gravity_up(&resources, position);

// Up according to the strongest single source, ignoring the rest.
let up = gravity_dominant(&resources, position);
```

`gravity_up` is the default and the one the camera's `UP_GRAVITY` mode uses.
Between two planets of similar pull it points at neither, which is correct
and reads as a character standing at a slant in open space.

`gravity_dominant` snaps to whichever source is winning. It is for
orientation — which way a character's feet point — and **never for a
force**: moving something with it applies a pull the solver is not applying.

## Seeing the fields

A gravity field has no mesh, no surface and no contact. An `AreaGravity`
rotated ninety degrees looks exactly like one that is not, until something
falls sideways.

So every source draws itself in the editor, in violet: the radii, the boxes,
the plane's heights, and arrows saying which way the pull goes. Direction is
drawn; magnitude is not — an arrow scaled by 9.81 would be a building, and
the strength is a number in the Inspector, where a number is a perfectly good
way to read a number.

## What this is not

Kóoch does not run a second solver. Gravity is acceleration summed here and
handed to rapier as an impulse of `mass × acceleration × dt` — instantaneous,
exactly equivalent to that force over the step, and composing with whatever
the game applies. Nothing here integrates a position.

Sleeping bodies are skipped unless the field itself changed. That is the
whole reason a settled scene stays cheap: rapier excludes a sleeping body
from the island solver, and every impulse wakes what it touches. A field
that pulled on all of them every step would keep the world simulating
forever.
