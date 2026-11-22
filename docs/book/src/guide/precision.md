# Large worlds and precision

A `Transform`'s position is an `f64` (`DVec3`): exact to well under a millimetre anywhere in a
solar system. Rotation and scale stay `f32`. Everything downstream — `GlobalTransform`, the GPU,
physics, the camera rig — works in `f32`, measured from a **world origin** that is never far away.

## The world origin

`WorldOrigin` is a cell of 2048 m. It follows:

- in Play, the entity with a **FloatingOrigin** (usually the player); the cell shows on that
  component;
- in the editor, the editor camera;
- nothing at all → it returns to zero.

When what it follows strays more than one cell, the origin jumps by whole cells. No `Transform`
moves: every `GlobalTransform` is republished from the exact `f64` positions, and `OriginShift`
says by how much that frame.

## Writing a system that stores a position

| You have | You want | Write |
|---|---|---|
| a root's `Transform` | what physics, rays, `gravity_up` and `teleport` take | `transform.relative_to(WorldOrigin::of(resources))` |
| an origin-relative point (a `GlobalTransform`, a ray hit, a solver pose) | a root's `Transform.position` | `WorldOrigin::of(resources) + point.as_dvec3()` |
| an origin-relative position kept across frames | still right after a rebase | add `OriginShift::of(resources)` when it is `Some` |
| a position kept across frames | never wrong | keep it as `DVec3`, absolute |

A child's `Transform` is local to its parent and small: it needs none of this.

## Scenes

Scenes saved with `f32` positions load unchanged; saving writes `f64`.
