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

**The ones built from a mesh.** These name a mesh asset and the engine
resolves it; they cannot be typed in.

| Shape | What you get | For |
|---|---|---|
| **Convex — one hull** | The mesh shrink-wrapped. Hollows fill in. | A dynamic prop whose visual mesh is too heavy. **The answer nine times out of ten.** |
| **Convex — several pieces** | Convex parts that together keep the hollows. | A concave prop where one hull would fill in the gap the design relies on. Expensive to derive — bake it. |
| **Complex — exact, static only** | The triangles themselves. | Static level geometry. **Wrong for anything dynamic** — see below. |

### What is not in the list

`Segment`, `Triangle`, `Polyline`, `Voxels` and `Voxelised mesh` build,
collide and are tested, and none of them answers a question an author
actually has — offering them made the dropdown a quiz.

They keep their discriminants and lose their labels, the same treatment
`Heightfield` has: **a scene authored with one still loads, still
resolves, and still shows its fields.** Nothing new picks one up by
accident. They come back the day something needs them — the voxel shapes
when terraforming does.

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

## Baking one from the Inspector

Select a mesh in the Asset Browser and its import settings grow two
buttons:

- **Create hull mesh** — one convex hull.
- **Create convex parts** — a convex decomposition, for a concave prop
  where a single hull would fill in the gap the design relies on.

Both write a `.glb` into `<project>/assets/collision/`, and both appear
in the same picker `Collider.mesh` uses. Point the collider at the result
yourself: the bake is a new asset with its own GUID, and nothing
repoints a collider behind your back.

**Why a file and not a cache.** The hull is cheap enough to derive at
load — 33 ms for a 76 000-vertex mesh, and the result is kept. The
decomposition is not: 1.35 s for Suzanne, 2.58 s for that same dragon,
and it runs again every time the body is rebuilt, which a scale drag
does. Baked, it loads as pieces and VHACD never runs.

A file also buys two things a cache cannot. You can open it in Blender
and see what the solver will collide against. And it can be simplified
*below* the exact hull, which nothing at runtime is allowed to do on its
own.

### `max faces`

Zero keeps the exact hull, which is what qhull already reduces to — 76 038
vertices come back as 387 — and is correct if dear. A budget simplifies
and then re-hulls: `meshopt` collapses edges with no reason to keep the
result convex, and a nearly-convex collider is one with a dent the solver
will find.

387 points is roughly 770 planes. For a dynamic prop that is a lot; other
engines cap around 255 vertices for the same reason.

### A bake remembers where it came from

The sidecar records the source's GUID and a hash of its bytes, and the
Inspector says so when the source has moved on:

> The source changed since this was baked — re-bake it

Without that a derived asset is a silent trap. Change the source mesh and
the bake keeps its own GUID, nothing fails, and the prop goes on
colliding with the shape it had last week.

Re-baking overwrites in place and **keeps the GUID**, so every collider
already pointing at it stays pointing at it.

## The mesh has to arrive first

A mesh-derived collider names a GUID, and the engine resolves it through
the asset database. Until that resolves, **the body is not created** —
deliberately. Substituting a unit sphere for a level's collision would
be a floor nobody authored, in a place nobody looks.

The moment the mesh lands, the collider is built. Nothing needs to be
reloaded and the scene needs no second save; a body that briefly does
not exist while a project starts is expected.

Reading the mesh does **not** go through the render pipeline. The
collider parses the `.glb` for positions and indices and stops there:
asking the asset server for a `MeshletMesh` would build the whole
simplification chain — 2.9 s for a 76 000-vertex mesh — and then decode
LOD 0 straight back into the triangles it started from.

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

A convex hull and a set of convex pieces get outlined from the same
points the solver was handed — which is the whole reason to draw them:
they are exactly the shapes that *differ* from what you can see.

A triangle mesh draws nothing, and that is not an omission. It **is** the
render mesh, edge for edge, so an outline would be a second copy of what
is already on screen at a hundred thousand lines a frame.
