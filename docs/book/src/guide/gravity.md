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
        // What a body feels anywhere in reach — the same on the surface as a metre above it, so
        // the pull does not change under your feet.
        strength: 9.81,
        // How far the pull reaches at full strength. Not the planet's surface: past it, around
        // everything the player does near the planet. Zero is unlimited.
        radius: 50.0,
        // How far past `radius` it fades to nothing, so leaving is not a line.
        falloff: 10.0,
    });
```

The same shape of reach as every other bounded source — whole inside, fading across a band — which
is what lets a `GravityPriority` take over a planet's surroundings the way it takes over a room.

## A transform places a field, it does not resize one

Every distance a source carries — `half_extents`, `radius`, `rounding`,
`range`, `falloff` — is in **metres**. Turning the entity turns the field
and moving it moves the field, but scaling it does neither.

That is deliberate, and it was the other way round once. A field's space is
rigid, so a `range` of 20 means twenty metres wherever the entity is and
whatever its scale says. It used to mean twenty *local* units, which on an
entity scaled to 8 — the obvious way to make a visible cube planet — pulled
from a hundred and sixty metres away, with the Inspector still reading 20.

So resize a zone by editing its extents, not by dragging a scale handle. A
gravity field is not geometry you eyeball; it is a number that has to mean
what it says.

If you want the source to sit on the same entity as a scaled mesh, it can:
the scale is simply ignored. Keeping them apart is still tidier, and it is
what the `gravity_tour` scene does.

## Sizing a planet

A body stays on a curved surface only while gravity covers the centripetal
acceleration its own speed demands. Past that it does **not** fall off — it
*orbits*, circling without touching. With `r` the body's centre — the
planet's surface plus the body's own radius — and `g` the `strength`:

```
stays on the ground   v ≤ √(g · r)
leaves for good       v ≥ √( g · (2·(radius − r) + falloff) )
```

Leaving for good is climbing out of the reach: the whole pull out to
`radius`, half of it across the fade. With a zero `radius` the field has no
end, and nothing leaves.

Earth's `√(g·r)` is 7.91 km/s, which is why none of this comes up on a flat
level: a sprinter is 791× below it. On a seven-metre planet a rolling ball
is *at* it.

**Speed is squared, radius is not.** Holding `v` needs `r ≈ v² / g`, so
doubling the top speed needs four times the planet. At Earth gravity, 8 m/s
wants a centre 6.5 m out and 20 m/s wants 41 m. A game about running fast on
small worlds is not a game about Earth gravity, and no amount of tuning
makes it one.

### Gravity is not a free knob

Raising `strength` to hold a faster body costs jump height, because both
come from the same `g`:

```
h = (J/m)² / (2·g)
```

A 6 N·s jump on a 1 kg ball of radius 0.5, on a planet whose surface is 4 m
out:

| `strength` | holds | jumps |
|---|---|---|
| 9.81 | 6.64 m/s | 1.83 m |
| 18 | 9.00 m/s | 1.00 m |
| 25 | 10.61 m/s | 0.72 m |

Growing the planet instead keeps both: a surface 7 m out at 9.81 holds
8.58 m/s and still jumps 1.83 m.

### A recipe

1. **Pick the feel**: what is the body's top speed?
2. **Pick the look**: how big should the planet be?
3. **Solve for the third** with `v = √(g·r)`. If the answer is a `strength`
   far from 9.81, check the jump height before accepting it.
4. **Set `radius` past the play volume around the planet.** Beyond
   `radius + falloff` the field stops and `gravity_up` falls back to world
   up, so the controls quietly become world-relative — which reads as "the
   gravity broke".

None of this applies to `AreaGravity`, `PlaneGravity` or `GlobalGravity`.
They are uniform: there is no curve to fall off, so any speed stays.
`BoxGravity`'s faces are flat and behave the same — its *edges* are where a
fast body launches, and `rounding` is the dial that softens them.

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

So give an overriding zone a soft edge. Every bounded source has a `falloff`
for exactly this — `AreaGravity`, `BoxGravity`, `PlaneGravity`, and
`PointGravity` past its `radius`. Zero is a hard edge, which a priority turns
into a jolt: the planet takes over all at once. `GlobalGravity` reaches
everywhere at full strength, so raising *it* switches off the rest of the
scene entirely.

Sources at the same level sum, as they always did. A source with no
`GravityPriority` sits at level 0, so adding the component to one entity
changes nothing about the others.

It works the other way round too, and on **every** kind of source: the
component goes on whichever entity should win. A planet inside a bigger
field — a level-wide `PlaneGravity`, a room — is the same move with the
priority on the planet. Inside its `radius` only the planet pulls; outside,
the field is back exactly as it was. Without it the two sum, and the floor's
down drags everything on the planet sideways. Nest as deep as you like: each
level overrules the ones below it only where it reaches.

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
