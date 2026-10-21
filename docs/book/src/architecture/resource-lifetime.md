# Who owns an asset's lifetime

Nobody, until #1589. `sync_scene_to_ecs` despawned entities and touched no asset
cache and no GPU pool, so every mesh and texture a session ever loaded was held
until the process ended. Loading a second scene cost the sum of both.

## One rule: what nothing is drawing goes

A resource is freed when nothing has asked to draw it for `MAX_AGE` frames.
That single rule covers what look like three separate cases:

| Case | Why it is the same event |
|---|---|
| a renderer is hidden | it is not in the ask |
| an entity is despawned | it is not in the ECS, so not in the ask |
| the scene changed or closed | `despawn_all` ran, so none of it is in the ask |

There is no unload path to write, and therefore none to forget to call. This is
the arrangement the shadow page pool has always used: pages carry the frame they
were last requested and are evicted past `DEFAULT_MAX_AGE`.

## Where the ask comes from

| Resource | The ask | Swept in |
|---|---|---|
| mesh pool slots + `Assets<MeshletMesh>` | `collect_referenced_guids` — visible `MeshRenderer`s | `sync_assets_to_gpu` |
| material textures + `Assets<Image>` | textures named by **drawn** materials: a visible `MeshRenderer`'s, or a `PostEffect` that is on and weighted above zero | `sync_textures` |

🔴 **The AssetDatabase is not an ask.** It lists every material file in the
project. Textures used to be uploaded from it, so texture VRAM was a function of
the project rather than the scene, paid whether or not anything drew them. The
upload is demand-driven now, which is what makes the sweep mean anything: ageing
without it would evict exactly what the next sync re-uploads.

## Two things that will bite

🔴 **`MAX_AGE` is the defence against thrash, not a tuning knob.** Evicting on
the first unasked frame would re-upload a mesh every time a renderer blinks,
which is the reallocation storm #1558 measured and removed. Sixty frames is one
second: long enough that a gesture is a decision.

🔴 **`GlobalMeshPool::release` hands the `mesh_id` out again on the next
`register`.** The sweep runs before the frame's instances are built and drops
the GUID from `MeshletPipeline::registry` in the same breath, so nothing is left
resolving a slot that now belongs to something else. A sweep placed after
instance building would draw the wrong mesh.

## What does not shrink, by design

Not leaks, and "fixing" them would undo measured work:

- Buffers carry `GROWTH = 2` slack and never shrink (#1558).
- The shadow page atlas is a fixed pool, 128 MiB at the defaults, allocated once
  and reused.
- Hi-Z pyramids retire through a triple-buffered queue: Mesa radv invalidates
  bind groups dropped while in flight.

## Still open

Materials themselves, scene documents and shaders still sit in `Assets<T>`
forever, and the frustum level of the ask — the cull decides visibility on the
GPU, so asking it means the GPU writes the ask, the way `PAGE_ASKED` does, never
a per-frame readback. See #1589.
