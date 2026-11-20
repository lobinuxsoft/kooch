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

Every planet in the scene shares one pool of 6144 voxel blocks, 68 MiB of GPU memory. The pool
never grows: each frame the blocks go where they do the most good on screen, so the planet you stand
on takes what the distant ones do not need. A planet too small on screen to matter gets none.

The blocks are refined for the camera closest to them. In the editor that is the scene view and
every active camera of the game, so the Game tab is as sharp as the Scene tab.

A frame sends at most one batch of 32 blocks to the GPU and never waits for it: a jump to another
planet fills in over a few frames instead of stopping one. The **Planets** section of the
Performance tab shows how full the pool is, how many blocks wait and what the last batch cost the
GPU.

## Not yet

- **Collision is still yours.** The ground has no collider yet; a sphere `Collider` at `radius` is
  the stand-in until it does (#313).
- **Seams between detail levels** can show a crack where a fine block meets a coarse one.
- The ground is meshed with surface nets: smooth, without the sharp edges dual contouring keeps.
