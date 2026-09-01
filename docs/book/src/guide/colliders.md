# Colliders

A `Collider` is what an entity presents to the solver: a shape, the
surface that shape has on contact, and the filtering that decides which
pairs are considered at all. It is the geometry — a `PhysicsBody` beside
it is what makes that geometry move.

Only the fields the selected shape reads are shown. Hiding is display
only: every field is still stored and still saved, so switching shape
back and forth never loses the other one's numbers.

## Picking a shape

**The cheap ones, in the order you should want them.** A primitive is
exact, has correct inertia, and costs the narrowphase almost nothing.

| Shape | Reads | For |
|---|---|---|
| Sphere | `radius` | Anything roughly round. The cheapest shape there is. |
| Cuboid | `half_extents` | Crates, walls, platforms. The only shape that scales exactly on every axis. |
| Capsule | `radius`, `half_height` | Characters. Rounded ends do not catch on the seams between floor tiles. |
| Cylinder | `radius`, `half_height` | Barrels, pillars, wheels. |
| Cylinder (rounded rim) | + `border_radius` | The same, where the rim keeps snagging on box edges. |
| Cone | `radius`, `half_height` | Spikes, funnels. |
| Half-space | `normal` | An infinite ground plane. |
| Segment, Triangle | `point_a`…`point_c` | Degenerate, and occasionally exactly right — a rail, a single ramp face. |

**The ones built from a mesh.** These name a mesh asset and the engine
resolves it; they cannot be typed in.

| Shape | Needs | For |
|---|---|---|
| Convex hull | vertices | A dynamic prop whose visual mesh is too heavy. The standard answer. |
| Convex decomposition | triangles | A concave prop where a single hull would fill in the gap the design relies on. Expensive to build. |
| Triangle mesh | triangles | Static level geometry. **Wrong for anything dynamic** — see below. |
| Polyline | vertices | A wire, a rail, a boundary. No volume. |
| Voxels | vertices | The cells the mesh's vertices land in. |
| Voxelised mesh | triangles | The mesh rasterised into cells at `voxel_size`. Collides against the cells directly, so it has no seam ghost-collisions. |

### The half-space is the floor you want

A test scene usually gets its ground from a cuboid scaled up until
nothing can be walked off it — a shape whose only job is to be large.
A half-space is the plane itself: it has no edges, so there is nothing
to fall past.

```
shape:  Half-space (infinite plane)
normal: (0, 1, 0)
```

### A triangle mesh is not a shape for a dynamic body

It has no interior, so it has no volume and no inertia tensor. Rapier
will build it and the solver will do something, but a dynamic body on
one tumbles wrongly and slides through its own shared edges. Use it for
level geometry that never moves, and give the moving thing a hull.

### `voxel_size` is the cost knob

Halving it multiplies the cell count by eight. The voxel shapes only
beat a triangle mesh while they stay coarse — start at a tenth of the
prop and go finer only if the collision visibly misses.

## The mesh has to arrive first

A mesh-derived collider names a GUID, and the engine resolves it through
the asset database. Until that resolves, **the body is not created** —
deliberately. Substituting a unit sphere for a level's collision would
be a floor nobody authored, in a place nobody looks.

The moment the mesh lands, the collider is built. Nothing needs to be
reloaded and the scene needs no second save; a body that briefly does
not exist while a project starts is expected.

A GUID that never resolves logs a warning naming it. That warning is the
only clue a collider is missing, so it is worth reading:

```
a collider names a mesh that will not load, so its body will not collide
```

## Surface and filtering

`friction` and `restitution` are this collider's own. How they combine
with the *other* collider's is a separate choice, and **the pushier
claim wins**: rapier resolves a pair by taking the higher of the two
rules, so a surface set to Average meeting one set to Max gets Max. A
rule is less "how my surface behaves" than "how I insist on being
combined".

Four masks decide what meets what:

- `collision_memberships` / `collision_filter` — whether the pair is
  considered at all.
- `solver_memberships` / `solver_filter` — whether, having been
  considered, it is pushed apart.

A pair is live only when **each** side's memberships intersect the
other's filter. Being in a group the other side looks for is not enough
on its own.

The two pairs existing separately is the point: a projectile that should
*detect* a wall without being *stopped* by it shares the wall's
collision groups and not its solver groups.

`sensor` is the other half of that idea — it reports overlaps and never
pushes. A sensor is not a collider that gets ignored: rapier computes no
contact manifold for it at all, so its events carry no contact
information.

## Several shapes, one body

A child entity carrying a `Collider` but no `PhysicsBody` of its own
contributes its shape to the nearest ancestor that has one, at its own
local offset and rotation. That is how a character gets a body capsule
plus separate hitboxes, and it is the configuration every engine
supports.

Two bodies that both simulate is the configuration none of them does —
the solver and the transform hierarchy would both own the child's pose.
If both really have to simulate, join them with a `Joint` instead. The
engine warns when it finds a dynamic body under another.

## What the gizmo shows

The outline is the *effective* shape, with `Transform.scale` folded in
the same way the solver folds it, drawn at `center` rather than at the
entity origin. An outline drawn from the authored numbers would lie
exactly where a collider is most likely to be wrong.

The mesh-derived shapes draw nothing yet. A wrong outline is worse than
none in the one tool that exists to tell the truth about this.
