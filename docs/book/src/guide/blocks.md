# Blocks

Blocks are level geometry you build **in the editor**: boxes, stairs, ramps, arches, doors, then
pulled and reshaped face by face — what ProBuilder is to Unity. A block is meant for blocking out a
level and for the simple architecture that stays in it; a hero prop still comes from a modelling
tool.

Blocks need the `blockmesh` feature.

## The geometry is in the scene

A block's mesh is the `Block` component itself, saved inside the scene — not a file in
`assets/`. A blockout belongs to one level, each copy has to be free to diverge from the next, and
the level stays one file to version and review.

From that mesh the engine generates two things, and regenerates them when you edit it:

- **the render mesh**, which the `MeshRenderer` beside it draws;
- **the collider**: a `Collider` on its own mesh, welded so a character does not catch on the
  seams, with a static `PhysicsBody`.

So a block is solid from the moment you place it. Switch the body to Kinematic and move its
`Transform` for a lift or a door. Dynamic is not for blocks: the collider is a triangle mesh, which
the solver does not simulate well as a falling body ([Colliders](colliders.md)).

## Placing one

Right-click the empty part of the World panel — or use the toolbar's Spawn button — → **Block**,
then a shape:

| Shape | What it is |
|---|---|
| Cube | A box (`size`) |
| Stairs | Steps rising along +Z (`steps`, `width`, `rise`, `run`) |
| Ramp | One slope over the stairs' footprint |
| Arch | A half ring on its feet |
| Cylinder | A prism |
| Cone | A prism collapsing to a point |
| Plane | A floor slab |
| Door | A rectangular door frame |

The parameters live on a `BlockShape` component beside the block, and the Inspector reshapes it
while you tune them. **Edit the geometry by hand and the shape lets go**: the parameters describe
what the block was generated from, and there is no way back from a hand edit to a number.

## Editing faces, edges and corners

A click inside a block can mean four things. Switch with the toolbar or the number keys:

| Key | A click selects |
|---|---|
| `1` | The whole entity — what every other entity wants |
| `2` | Corners |
| `3` | Edges, moving the two corners at their ends |
| `4` | Faces |

Then move what you selected with the usual handles (`W`, `E`, `R`). **`Shift+E` extrudes** the
selected faces — pulls them out with new sides. It is not plain `E`, because that is the rotate
handle and one key with two meanings made every extrude also switch mode.

Every edit is one undo step.

### Along a curve

With faces selected, the Inspector's **Extrude along a curve** runs them down any entity's
`Spline`: pick the curve, how many `spans` per segment, and whether each knot's radius should
`taper` the section. A wall that follows a road, a gutter along a roof.

## Building from a spline

Two components turn a `Spline` into geometry. Both sit on the entity that already has the spline,
so the knots, tangents and radii are authored once.

**`SplineSweep`** carries a profile along the curve:

| Field | Does |
|---|---|
| `profile` | Round (a pipe), Square (a beam, a kerb), or Steps |
| `sides` | Corners around a round profile |
| `spans` | Rings per curve segment — how smooth a bend is |
| `caps` | Close the ends |
| `wall` | Thickness. 0 is solid; above it, a tube you can walk through |

The knots' radius is what tapers a trunk or narrows a pipe. **Steps** puts one level step per span,
with the curve's climb as the rise: straight, curved and spiral stairs are the same curve.

**`SplineDeform`** bends an existing mesh asset along the curve — Blender's Curve modifier.
`source` is the mesh, `axis` its length axis, `taper` scales it by the radii. The asset is read,
never written; the bent copy is generated, with its own collider.

## Reading blocks from a game

`Block`, `BlockMesh`, `SplineSweep` and `BuiltBlocks` are in the prelude. A game reads a block's
geometry from `Block.mesh`; `BuiltBlocks` is the engine's record of which blocks it has already
turned into a mesh.
