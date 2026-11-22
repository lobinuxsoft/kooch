# Large worlds and precision

A `Transform`'s position is an `f64` (`DVec3`): exact to well under a millimetre anywhere in a
solar system. Past that, a root carries a **GridCell** and its position is local to the cell.
Rotation and scale stay `f32`. Everything downstream — `GlobalTransform`, the GPU, physics, the
camera rig — works in `f32`, measured from a **world origin** that is never far away.

## The world origin

`WorldOrigin` is a cell of 2048 m. It follows:

- in Play, the entity with a **FloatingOrigin** (usually the player); the cell shows on that
  component;
- in the editor, the editor camera;
- nothing at all → it returns to zero.

When what it follows strays more than one cell, the origin jumps by whole cells. No `Transform`
moves: every `GlobalTransform` is republished from the exact `f64` positions, and `OriginShift`
says by how much that frame.

## Sectors: GridCell

A `GridCell` (three `i64`) puts a root in a 2048 m cell; its `Transform.position` is then measured
from that cell's corner. A cell index reaches 10²² m, far past any galaxy, and stays exact.

- Something dropped in the viewport more than 4·10⁹ m out (`FAR`) gets one by itself.
- While playing, a root that strays past a cell moves whole cells from its position into its
  `GridCell`. Nothing moves on screen.
- Planet blocks carry their planet's cell.
- Below a root, a `GridCell` is ignored.

## Writing a system that stores a position

Take `let origin = WorldOrigin::get(resources);` once, then per root `entity`:

| You have | You want | Write |
|---|---|---|
| a root's `Transform` | what physics, rays, `gravity_up` and `teleport` take | `origin.relative(registry, entity, transform)` |
| an origin-relative point (a `GlobalTransform`, a ray hit, a solver pose) | that root's `Transform.position` | `origin.local(registry, entity, point)` |
| an origin-relative point | a **new** root's place | `origin.place(point)` → `(Option<GridCell>, DVec3)` |
| an origin-relative position kept across frames | still right after a rebase | add `OriginShift::of(resources)` when it is `Some` |

These go through the root's `GridCell`, or cell zero without one. `WorldOrigin::of(resources)` is the
cell-zero shortcut, right only for a root without a `GridCell`. A child's `Transform` is local to
its parent and small: it needs none of this.

## Scenes

Scenes saved with `f32` positions load unchanged; saving writes `f64`. Cells saved as `i32` load
as `i64`.
