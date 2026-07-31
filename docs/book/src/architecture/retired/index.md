# Retired

Pages describing code that no longer exists.

They are kept because the reasoning in them was real and cost something to arrive at, and
because a decision is easier to revisit when you can still read what it replaced. **Nothing
here describes the engine as it is.**

## SDF ray-marching and its BVH

The engine's original rendering path was signed-distance-field ray-marching, accelerated by a
BVH that several consumers shared. Both crates — `kooch_sdf` and `kooch_bvh` — were deleted in
July 2026.

The technique died; **the data did not**. Signed distance fields remain the representation
behind the voxel and dual-contouring work, where they are extracted to meshes that go through
the same GPU-driven meshlet pipeline as everything else. What was retired is the *renderer*
that marched them directly.

The current path is described in [Render Pipeline](../render-pipeline.md).
