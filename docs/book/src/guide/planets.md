# Planets

A planet is one component: put a `Terrain` on an entity and the engine draws a ground you can walk
around, generated on the GPU from the planet's shape. Its centre is the entity's position, and it
turns and moves with the entity without regenerating anything.

Terrains need the `terrain` feature (crate `kooch_terrain`). A scene that uses one turns it on for
the game build by itself. The older names still work: a manifest that asks for `planet`, and scenes
saved when the component was `kooch_planet::component::Planet`. Saving writes the new name. The base
shape is a sphere or a plane, and the generator picks it: see [Flat terrain](#flat-terrain).

| Field | What it does |
|---|---|
| `radius` | Sea level of a sphere: where the ground is with no generator |
| `extent` | Half the side of a plane: the ground spans this far either side of the centre on X and Z. A sphere ignores it |
| `relief` | The highest the generator may raise the ground above `radius` |
| `depth` | The deepest a cave may reach below it; under it the core is solid |
| `voxel` | The finest detail the ground can have, next to the camera |
| `lod_distance` | How far from the camera the finest voxels reach; each coarser level reaches twice as far. The levels are chosen again only when the camera crosses into another finest block (16 voxels across), or something else they depend on changes |
| `generator` | A `terrain` shader — usually a graph — that shapes the ground. Empty is the bare sphere |
| `material` | What the ground is drawn with |

Lengths are in the entity's local units: keep its scale at 1 and they are metres.

## The shape is a shader

The ground is wherever the generator's signed distance crosses zero: negative is rock, positive is
air. Write it as a [terrain shader](shaders.md#terrain), or set a shader graph's Output kind to
`terrain` and wire **Sphere Base** and **Planet Direction** through noise. Save the graph and the planet
regenerates; a generator that does not compile draws the bare sphere and says why in the Console.

**`relief` and `depth` are a promise.** The engine looks for ground only between `radius − depth`
and `radius + relief`, so a mountain taller than `relief` is cut flat at the top. Bigger costs
blocks, so keep them as small as the shape allows.

**A bounded generator costs fewer blocks.** Before giving a block a slot, the engine runs the
generator on the CPU over the whole block at once and skips it when the result cannot cross zero.
`clamp`, `min`, `max`, `smoothstep` and noise that stays in a known range all help; a `tan`, a loop
whose length depends on the position or a texture read make every block in the band a candidate.

## Flat terrain

A level that is not a planet is the same component over a plane. In the generator's graph, use
**Plane Base** where a planet uses **Sphere Base**: it is the height above the terrain's local XZ
plane, and **Planet Direction** reads `+Y` there. Every relief recipe below works unchanged. The
ground spans `extent` either side of the entity on X and Z, and `relief` and `depth` bound it in Y;
streaming, caves and the ground colliders work as on a planet.

A graph holds one base node: with both, it does not save. The node is how the engine knows the
shape, through a `// base: plane` line the graph writes; a hand-written generator adds that line
itself. A plane has no core, so nothing under it is hidden from the camera the way a planet's far
side is. Gravity is the project's, not the terrain's.

## Shaping the relief

One noise gives hills everywhere. A planet with somewhere to go is a few height layers in metres,
combined, and subtracted from **Sphere Base** at the end:

| Feature | Graph |
|---|---|
| Continents and ocean floor | a low-scale noise on **Planet Direction** through a **Curve** that is flat across the coast — the flat stretch is the beach |
| Mountain ranges | a **ridged** noise, multiplied by the continent value so ranges stay inland |
| Valleys | what lies between ranges once they are masked; a **Curve** that dips before rising widens them |
| Canyons | a ridged noise turned into a floor (One Minus, times the depth) and joined to the height with **Smooth Min**, so the rim is rounded, not a crease |
| Mesas, stepped slopes | **Terrace** on a layer, `softness` near 0.3 |

Two limits keep the ground smooth:

- **A wave needs two voxels.** The finest octave is `scale × 2^(octaves − 1)` waves per unit of
  direction; at `radius` that is a wavelength of `radius / (scale × 2^(octaves − 1))` metres. Under
  twice the planet's `voxel` the mesher draws it as teeth.
- **`distortion` folds.** A warp strong enough to fold the noise onto itself leaves knife ridges in
  the field, which no shading hides. Raise it a little at a time.

## What the ground is made of

The generator also says what the ground is made of: grass, rock, snow. Each layer is a **Layer** node
in the generator's graph, numbered 0..31, with its own rule wired into its `weight`. Weights need
not add up to anything — only how they compare counts — and two nodes of one layer add. Without any
Layer node the ground is layer 0.

Rules are graph math. **Slope** is how steep the ground is, from 0 on the flat to 1 on a wall and 2
under a ceiling; the height is the base node you already have. Rock on cliffs is
`Layer 1 = Smoothstep(0.3, 0.5, Slope)`, snow above forty metres `Layer 2 = Smoothstep(38, 42, Sphere
Base)`, and grass whatever is left, `Layer 0 = One Minus` of the two. Slope only reaches Layer
nodes: it is measured on the distance, so the distance cannot read it.

Each sample keeps its **four heaviest layers**, as Godot Voxel does, so up to four blend where biomes
meet. What a layer *looks* like is the surface material's choice: in the graph of the `Terrain`'s
`material`, **Layer Weight** gives how much of one layer shows at each point, 0..1, and the colour is
each layer's look times its weight, added. A texture per layer, past the four a material binds
today, arrives with #2047.

The material is measured on the surface, with the slope over the finest `voxel`, so a block looks
the same at every level of detail: only how fine the pattern is changes. That costs seven generator
calls per sample near the surface, and none for the rest. Each sample stores its layers in 4 bytes,
twice its distance: the material atlas is 137 MiB at the default 6144 blocks. On the CPU,
`Generator::material` answers the same question at any point, for footsteps or friction.

## Stamps

A stamp drops a shape on the ground without touching the generator: a hill, a crater, a mesa. They
live in the `Terrain`'s **stamps** list and apply in list order, first to last, over what the
generator made. Nothing is baked: move one, reorder the list or switch one off, and the ground
follows.

Each stamp has a **shape** (sphere, box, capsule, torus, cone, volume or field), an **operation** and a
placement:

- **Add** raises rock, **Subtract** digs, **Intersect** keeps only what is inside the shape (it
  reaches the whole terrain, so changing one regenerates all of it).
- **position** is the centre in the terrain's space, in metres. Its up is the ground's: away from a
  sphere's centre, or +Y on a plane. **turn** spins it about that up, in degrees.
- **radius** for a sphere; **size** for a box, half its side on each axis, y up. A **capsule** and a
  **cone** take a **radius** and a full **height** up the ground; a **torus** lies flat, **radius**
  to the middle of its **tube**.
- A **volume** stamps a `volume` shader (below), kept inside **radius**.
- **smooth** melts the edge into the ground over that many metres; 0 is sharp.
- **opacity** fades the stamp: 0 is the ground without it.
- **layer** and **paint** lay a material layer (see above) over the surface the stamp made, so a
  crater can be rock in a grass field.

With the terrain selected, every stamp is drawn as its shape (green adds, red digs, blue
intersects, grey is off) with a grip at its centre. Click a grip and the transform gizmo moves the
stamp, turns it about the ground's up and sizes it along its own sides — its axes follow the
stamp's frame on the ground, not the world's. Hold Ctrl+Shift while moving to drop the stamp where
the cursor meets the ground. The shape follows the drag; the ground is regenerated once, on
release, and one drag undoes in one step.

A Terra-sized hill: `Add`, sphere, position `(0, 58, 0)`, radius `8`, smooth `3`. A pit next to it:
`Subtract`, box, position `(12, 60, 0)`, size `(3, 4, 3)`, smooth `1`.

### Volumes

A `volume` shader is a shape authored once and stamped anywhere: an arch, a mesa, a rock spire. Its
graph writes a signed distance in the stamp's own space, metres from its centre with y the ground's
up, and **Plane Base** is that `y`. The **engine:volumes** functions (`ball`, `block`, `torus`,
`capsule`, `cylinder`) are its nodes, reading **Planet Position**; **Smooth Min** joins two, and the
**Max** of one and `0 − other` (a **Subtract**) cuts the other out. Whatever it writes is kept inside the stamp's **radius**.

An arch: `block` centre `(0, 4, 0)` extent `(8, 5, 2)`, cut by `cylinder` from `(0, 0, -3)` to
`(0, 0, 3)` radius `5` — the Max of the block and `0 − cylinder` into the Output. Stamp it as a
volume of radius `10`, `Add`, on Terra; stamp it twice and the ball rolls under both.

A volume joins the generator of every terrain that stamps it, once: naming a new one recompiles
that generator, while moving, copying or reordering its stamps does not. A volume shader that does
not compile, or defines a helper differently from the generator, leaves its stamps off and says so in
the log.

### Fields

A **Field** stamp takes its shape from data (#709): a `.kfield` the editor bakes, stretched over the
stamp's box **size** (x and z along the ground, y up).

- **From a mesh:** select the mesh in the Asset Browser and press **Create SDF field** under its
  import settings. The mesh's signed distances, `resolution` texels along its longest side (64 by
  default), land in `assets/terrain/<mesh>_sdf.kfield`. An open mesh works: the inside is told
  apart by how many times the surface wraps a point, not by a ray. Stamped with `Add`, the mesh
  stands in the ground, overhangs and all; with `Subtract`, it carves its mould.
- **From an image:** select it and press **Create height field**. Its brightness is the height,
  black at the box's floor and white at its top; a 16-bit PNG keeps all of its precision. A larger
  image is averaged down to `resolution` texels along its longer side (256 by default). It lands
  in `assets/terrain/<image>_height.kfield`. Sink the box's floor below the ground, or the stamp
  shows its walls.

Suzanne as a rock: bake `suzanne.glb`, then a Field stamp on Terra with `size (4, 4, 4)`, `Add`,
smooth `1`. A dune field: bake a dune heightmap, `size (20, 3, 20)`, its centre a metre under the
ground.

The engine ships 54 heightmaps to start from in `assets/terrain/stamps` — dunes, hills, mountains,
volcanos, craters, cliffs, islands, lakes, ridges, rivers, roads — and 8 ground layers in
`assets/terrain/layers`. To see them all at once:

```text
cargo run --example make_atlas_stamps --features editor,terrain -- <parent dir>
```

writes an `AtlasStamps` project whose flat terrain carries one Field stamp per heightmap, in a grid.

The texels stay on the CPU as well and both sides filter them by hand, with the same weights, so the
ball collides with what is drawn. A field keeps its steepest slope from the bake, and the stamp scales
its value by it, so a squashed map stays a true distance for the range analysis. A field is 2 bytes a
texel: a 2048² map is 8 MiB, a 64³ volume 512 KiB. Every field in the scene shares one storage
buffer capped at 128 MiB; past it the editor logs an error and keeps the fields it had, so bake at
the resolution the voxels can show — 256 texels over a 100 m stamp is already finer than a 0.5 m
voxel.

A stamp is data the generate pass reads, not code: moving one recompiles nothing, and only the
blocks inside its old and new reach generate again, the rest of the planet as it was. A stamp
that reaches past `relief` or `depth` adds blocks only inside its own box: the planet's band stays
as authored, so a wide stamp on a large planet costs its own area, not a shell around the world
(#2055). The CPU twin applies
the same stack, so collision and the ground raycast follow every stamp; a volume is bounded by the
range of its own graph there, as a graph's distance need not be a true one. More shapes, and shapes
authored as graphs, come with #1957; shapes from a 3D texture or a heightmap with #709.

## What it costs

A planet is one entity however many blocks it shows: the blocks are drawn as instances of it, so
they never fill the World panel or cross to the editor. Clicking its ground in the viewport selects
the planet, and a prefab dropped on the ground lands on it, against the generator's exact surface
rather than the block that happens to be showing.

Each block draws three meshes: its interior, made once from its own samples; its seam, the shell
that meets its neighbours; and an overlap, a skirt that leaves the interior's edge and sinks under
the seam. When a neighbour changes level only the seam is made again, and while it is on its way
the overlap covers the crack, from any angle. A new block appears once all three are ready.

Every planet in the scene shares one pool of 6144 voxel blocks, 68 MiB of GPU memory. The pool
never grows: each frame the blocks go where they do the most good on screen, so the planet you stand
on takes what the distant ones do not need. A planet too small on screen to matter gets none.

Detail falls with distance from the camera. The finest voxels reach the Terrain's **LOD
distance** (32 m by default), and each coarser level reaches twice as far, with voxels twice as
big. Raise it for more detail further out, at the cost of blocks. To see the blocks, set the Debug
view to **Instance Ids**: each colour is one block, kept while the block lives, and they grow with distance.

The blocks are refined for the camera closest to them. In the editor that is the scene view and
every active camera of the game, so the Game tab is as sharp as the Scene tab.

A frame sends at most one batch of 32 blocks to the GPU and never waits for it: a jump to another
planet fills in over a few frames instead of stopping one. The **Planets** section of the
Performance tab shows how full the pool is, how many blocks wait and what the last batch cost the
GPU.

## Walking on a planet the size of Earth

A `Transform`'s position is double precision, exact to well under a millimetre anywhere in a solar
system, and the inspector edits it as such. What reaches the GPU, physics and the camera is single
precision, which steps half a metre at Earth's radius, so it is measured from a **world origin**
that follows the view: whenever the view strays more than 2048 m away, the origin jumps by whole
2048 m cells. Nothing authored moves — only what is drawn and simulated is re-centred.

While the game plays, the origin follows the entity with a **FloatingOrigin**, usually the player;
the cell it is in shows on that component. While editing, it follows the editor camera, so a planet
can be edited at its surface as precisely as near zero. A scene without a FloatingOrigin plays with
the origin at zero, as before, and scenes saved with single-precision positions load unchanged.

In the Edit View, **F** on a planet frames the whole sphere; scrolling backs off as far as the
largest planet needs, and flying speeds up with the distance to what you orbit.

Code that keeps an origin-relative position across frames reads `OriginShift` and adds its offset
on the frame the origin moves. Code that writes a root's `Transform` from a world-space value — a
`GlobalTransform`, a ray hit, a physics pose — adds `WorldOrigin::of(resources)` first.
The other way round, `transform.relative_to(WorldOrigin::of(resources))` is the position physics
rays, `gravity_up` and `teleport` expect.

## Standing on the ground

Only the ground near a moving body collides. The block every dynamic or kinematic `PhysicsBody`
is in, and the 26 around it, are generated by the same GPU pass that draws them, read back, meshed
on worker threads, and welded into one static trimesh per planet, so crossing into a block finds its
neighbours already built. A block is released once no body is within two blocks of it; the ring in
between is kept so a body rolling back and forth over an edge rebuilds nothing. Blocks with no
surface are remembered and never generated again. Where no GPU is running a frame, as in the
editor's Play host, a compute-only device is created for it. Nothing is built in edit mode.

- **No bumps at block seams.** The blocks are one mesh to the solver, so the edges between them
  are smoothed like any other; separate meshes would kick a rolling body at every seam. The weld
  and the build run on a worker, and the new shape replaces the old one on the same body, so a ball
  rolling on it keeps its contact instead of landing again on every rebuild.
- **A body waits for its ground.** Until the ground within 1.5 m of it exists, the solver holds it still
  instead of letting it fall through; spawning or teleporting onto a planet is safe.
- **Ground Invoker** sets how far ahead, in metres rounded up to whole rings of blocks: raise it
  for fast bodies, which can outrun the readback (16 blocks in flight). It also asks for ground on an entity with no body.
- The collider is a static body straight in the solver, not entities: nothing in the hierarchy,
  nothing saved with the scene, all removed when Play stops, rebuilt when the generator changes.
- **Seeing them:** in Play, the viewport's **Physics** menu → **Planet ground colliders** draws the
  triangles in the solver in green, blocks being generated or waiting for the weld as yellow boxes and empty ones as grey
  boxes. Each block's triangles cross to the editor once and stay on the GPU.
- Requires the `physics` feature.
