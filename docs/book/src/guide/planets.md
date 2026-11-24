# Planets

A planet is one component: put a `Planet` on an entity and the engine draws a ground you can walk
around, generated on the GPU from the planet's shape. Its centre is the entity's position, and it
turns and moves with the entity without regenerating anything.

Planets need the `planet` feature. A scene that uses one turns it on for the game build by itself.

| Field | What it does |
|---|---|
| `radius` | Sea level: where the ground is with no generator |
| `relief` | The highest the generator may raise the ground above `radius` |
| `depth` | The deepest a cave may reach below it; under it the core is solid |
| `voxel` | The finest detail the ground can have, next to the camera |
| `lod_distance` | How far from the camera the finest voxels reach; each coarser level reaches twice as far |
| `generator` | A `planet` shader — usually a graph — that shapes the ground. Empty is the bare sphere |
| `material` | What the ground is drawn with |

Lengths are in the entity's local units: keep its scale at 1 and they are metres.

## The shape is a shader

The ground is wherever the generator's signed distance crosses zero: negative is rock, positive is
air. Write it as a [planet shader](shaders.md#planet), or set a shader graph's Output kind to
`planet` and wire **Altitude** and **Planet Direction** through noise. Save the graph and the planet
regenerates; a generator that does not compile draws the bare sphere and says why in the Console.

**`relief` and `depth` are a promise.** The engine looks for ground only between `radius − depth`
and `radius + relief`, so a mountain taller than `relief` is cut flat at the top. Bigger costs
blocks, so keep them as small as the shape allows.

**A bounded generator costs fewer blocks.** Before giving a block a slot, the engine runs the
generator on the CPU over the whole block at once and skips it when the result cannot cross zero.
`clamp`, `min`, `max`, `smoothstep` and noise that stays in a known range all help; a `tan`, a loop
whose length depends on the position or a texture read make every block in the band a candidate.

## Shaping the relief

One noise gives hills everywhere. A planet with somewhere to go is a few height layers in metres,
combined, and subtracted from **Altitude** at the end:

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

## What it costs

A planet is one entity however many blocks it shows: the blocks are drawn as instances of it, so
they never fill the World panel or cross to the editor. Clicking its ground in the viewport selects
the planet, and a prefab dropped on the ground lands on it, against the generator's exact surface
rather than the block that happens to be showing.

Each block draws two meshes: its interior, made once from its own samples, and its seam, the shell
that meets its neighbours. When a neighbour changes level only the seam is made again, and a new
block appears once both are ready.

Every planet in the scene shares one pool of 6144 voxel blocks, 68 MiB of GPU memory. The pool
never grows: each frame the blocks go where they do the most good on screen, so the planet you stand
on takes what the distant ones do not need. A planet too small on screen to matter gets none.

Detail falls with distance from the camera. The finest voxels reach the Planet's **LOD
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

## Not yet

- **Collision is still yours.** The ground has no collider yet; a sphere `Collider` at `radius` is
  the stand-in until it does (#313).
- **Seams between detail levels** can open for a few frames while the camera moves and blocks
  change level; once they settle the ground is closed (#1999).
