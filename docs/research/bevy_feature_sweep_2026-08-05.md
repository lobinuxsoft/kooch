# What to take from Bevy — sweep of 0.12 → 0.19

Read on 2026-08-05: the release notes for 0.12 through 0.19, plus the
virtual-geometry write-ups, the BVH-culling PR and the `bevy_ecs` docs.

**Take the newest form of each feature, not the one it shipped as.**
Virtual geometry landed in 0.14 and is a different system by 0.17;
atmosphere landed in 0.16 and lights the scene by 0.18. The version
column below says where a thing *appeared*; what to port is where it
stands now.

**Everything here is evaluated against one goal: universes.** Draw
distances and techniques have to survive planetary and galactic scale.
Detail is only required up close — far away it only has to be
*distinguishable*, not detailed. A feature that looks good in a room and
falls over at 32 km is not a feature for this engine.

Nothing is copied. Designs are read and reimplemented against our ECS,
which is not theirs (`feedback_bevy_is_wgpu_ceiling` sets the ceiling;
this file sets the shopping list).

## 🔴 The finding that outranks the rest

Our visible-meshlet id is packed as `(instance_id << 16) | meshlet_id` in
a `u32` — see `meshlet_deferred.wgsl:204` and `meshlet_debug_resolve.wgsl:68`.

That is a hard ceiling of **65 536 instances** and **65 536 meshlets per
mesh**.

Bevy had a comparable limit — 2²⁴ clusters, about 4 billion triangles —
and **removed it in 0.17**. Their test scene is 130 000 dragons of
~870 000 triangles each: **115 billion triangles, 3.5 ms on a 4070**
(~3.1 ms geometry, ~0.4 ms material).

65 536 instances is a single vegetated chunk. This is the first thing
that will stop a planet, and it is ours, not inherited.

## Take, in priority order for universe scale

| # | What | Where it is in Bevy | Why it matters here |
|---|---|---|---|
| 1 | **Meshlet BVH culling** | 0.17, [PR #19318](https://github.com/bevyengine/bevy/pull/19318) (atlv24, SparkyPotato) | Render cost becomes *nearly independent of scene geometry*, and the cluster ceiling disappears. This is the single change that turns "a big scene" into "a planet". Culling walks a BVH over clusters instead of testing every cluster. |
| 2 | **Wider instance/meshlet ids** | consequence of the above | Our 16/16 packing has to go before anything else on this list means much. |
| 3 | **Visibility Ranges / HLODs** | 0.14 | Per-mesh appear/disappear distances. Cheap, and the honest answer to "far away only has to be distinguishable". |
| 4 | **Two-phase occlusion culling** | 0.16, folded into virtual geometry in 0.17 | We have the Hi-Z two-pass (#445) parked behind #486. Bevy shipped theirs and then absorbed it into BVH culling. |
| 5 | **Procedural atmospheric scattering** | 0.16, raymarched from above the atmosphere in 0.17, **affects scene lighting** in 0.18 | Exactly the planetary case: a sky that is correct *seen from orbit*, and sunlight that picks up its colour through the atmosphere. Feeds #250 and #248. |
| 6 | **GPU-driven rendering pass** | 0.16, ~3× on complex scenes | We are already GPU-driven in the meshlet path; worth reading for what they moved off the CPU that we have not. |

## Take, once scale is handled

| What | Version | Note |
|---|---|---|
| **PBR fixes: Fresnel, over-glossy materials** | 0.18 | Cheap correctness. Read before writing #441 rather than after. |
| **Contact shadows** (short screen-space ray) | 0.19 | Large visual return for the cost; independent of shadow-map work. |
| **PCF for point lights**, **PCSS** | 0.14 | Feeds #476. |
| **Volumetric fog + fog volumes + god rays** | 0.14, 0.15 | Fog volumes are bounded boxes with 3D density textures — composes with chunks. |
| **Auto exposure** | 0.14 | Not optional at planetary scale: sunlit surface to night side is orders of magnitude. |
| **Depth of field**, **motion blur**, **chromatic aberration**, **anamorphic bloom** | 0.14, 0.15, 0.16 | Post stack. Independent, do late. |
| **VBAO** (replaced GTAO) | 0.15 | Better on thin geometry. |
| **Order-independent transparency** | 0.15 | Per-pixel sorting. Relevant when foliage lands. |
| **Decals** (forward + clustered) | 0.16 | |
| **Specular tints and maps** | 0.16 | Feeds #441. |
| **Mesh tags** (per-instance `u32` for shaders) | 0.16 | Per-instance data without a new material — useful for planet/biome ids. |
| **Retained gizmos** (~65–80× over immediate) | 0.16 | Our gizmos are immediate; see `feedback_immediate_mode_per_frame_cost`. |
| **GPU timestamps in the profiler trace** | 0.16 | We have GPU timers already; theirs land in the trace beside CPU work. |
| **Fullscreen material trait** | 0.18 | Post-processing without ceremony. |
| **glTF extension handlers** | 0.18 | Custom data through the importer instead of beside it. |

## From 0.12 / 0.13 — the older ones still worth having

| What | Version | Note |
|---|---|---|
| **Automatic batching & instancing** | 0.12 | Up to 3× in their benchmarks. Draw commands merge when they can. |
| **Asset preprocessing (Asset V2)** | 0.12 | Meta files, multiple sources, recursive dependency tracking. We have `.meta` and a database; theirs also *preprocesses*, which is what #578 wants for baked meshlets. |
| **Deferred rendering** | 0.12 | Ours already is. |
| **Light transmission** (glass, water, foliage) | 0.12 | Foliage is the planetary case, not the glass. |
| **Lightmaps** | 0.13 | Precomputed GI for static geometry — the cheap far-field answer where surfels are too expensive. |
| **Irradiance volumes** | 0.13 | Voxel-sampled ambient. Rides the voxel chunks we already have; overlaps #450. |
| **Reflection probes with AABBs** | 0.13 | Multiple environment maps per region — per-biome, per-planet. |
| **Render assets unload from RAM after GPU upload** | 0.13 | Straight VRAM/RAM win at scale, no visual cost. |
| **Dynamic queries** | 0.13 | Runtime-defined filtering. Note their stated use case: *scripting and modding* — the consumer is third-party, not an engine scripting language. |

## Their ECS versus ours

`bevy_ecs` is usable on its own — *"it was created specifically for Bevy's
needs, but it can easily be used as a standalone crate"* — so the
question is fair rather than academic.

**What theirs has that ours does not:**

| | Why it matters here |
|---|---|
| **Change detection** | The one that matters at planetary scale. 0.16 got *11× faster transform propagation* out of dirty-bit skipping for static hierarchies. A planet is almost entirely static hierarchy. |
| **Explicit system ordering** (`before` / `after`, sets) | Exactly what #392 needs; our `add_system(stage, f)` orders by insertion. |
| **Parallel execution** | Our scheduler runs systems sequentially. |
| **Observers and hooks** | Push-based reaction to component lifecycle. Not scripting — plain ECS. |
| **Relationships** | Bidirectional entity links the ECS maintains. Compare `feedback_camera_target_tag`: we chose tags precisely to avoid dangling references, and relationships are the other answer. |
| **Sparse-set storage** | For components on few entities, or toggled often. |

**What ours has that theirs does not:**

| | |
|---|---|
| **GPU-resident components** | `ComponentRegistry` holds CPU *and* GPU storages. Bevy's render world is a separate `World` it extracts into every frame. Ours is the hybrid the engine is built on. |
| **Dynamic components from a `.so`** | `DynamicTypeRegistry` — a project's components, compiled in its own dylib, appear in the editor's menus and Inspector. Bevy has no editor to need this. |
| **Reflection wired to an editor that exists** | |

**Verdict: do not migrate. Port the three pieces that hurt.**

Migrating means rewriting every crate against a foreign `World`, losing
GPU-resident components and the dynamic registry — the two things that
make the editor and the hybrid design work. The cost is the engine; the
benefit is features we can port individually.

In order of what actually blocks us:

1. **System ordering** — needed by #392 regardless, and cheap.
2. **Change detection + dirty-bit transform propagation** — their 11× is
   on ordinary scenes. On a planet, where nearly everything is static and
   nearly nothing moves per frame, re-propagating everything is the
   difference between a planet and a demo.
3. **Observers** — only once something needs to react immediately rather
   than next stage. No consumer yet; do not build it early.

## Already ours, or ahead

| Their feature | Ours |
|---|---|
| METIS meshlet generation ([PR #16947](https://github.com/bevyengine/bevy/pull/16947)) | Already graph-based METIS — see `oh_my_engine_meshlet_grouping` |
| GPU frustum culling (0.14) | Have it |
| Two-phase Hi-Z (0.16) | #445 built, parked behind #486 |
| Virtual geometry (0.14+) | The whole meshlet path is ours |
| Render graph | **They deleted theirs in 0.19** — passes are systems now. That is #392 |
| `InputFocus` resource (0.16) | Written today, same shape, arrived at independently |
| **Large worlds** | 🔴 They have no coordinate solution in-engine — the ecosystem answer is the third-party `big_space` crate (floating origin, `i64` cells). **We have `ActiveOrigin` with sectors already.** On precision we are not behind, we are elsewhere and further along |

## Not for us

- **Solari** (0.17/0.19) — raytraced lighting, needs hardware RT. Our GI is surfel + voxel coupling (#450), chosen because it rides the chunk structure we already have.
- **`no_std`**, UI widgets (popovers, menus, colour pickers), `bevy_ui` transform work — different product.
- **WESL shaders** (experimental) — watch, do not adopt.
- **Retained render world / ECS relationships / entity cloning / immutable components** — their ECS, not ours.

## What this changes about our issues

To be applied next, not assumed:

- **New, top priority:** widen the meshlet id packing, then BVH culling over clusters. Nothing else on the universe path matters until scenes can exceed 65 k instances.
- **#476 (CSM), #441 (PBR):** read 0.18's Fresnel/glossiness fixes and 0.14's PCF/PCSS *before* implementing, not as a follow-up.
- **#250 (sky material):** widen to the 0.16→0.18 atmosphere arc — raymarched from orbit, and atmosphere feeding scene lighting.
- **#486 (Hi-Z orchestrator):** re-read against BVH culling. If clusters are culled through a BVH, the two-phase pass may want to be a phase *of* it rather than beside it.
- **#342 (impostor cubemap baking):** overlaps with Visibility Ranges. Decide which answers "far away is only distinguishable".
- **#392:** unchanged, and confirmed by 0.19 deleting theirs.

## Sources

- Release notes: [0.14](https://bevy.org/news/bevy-0-14/) · [0.15](https://bevy.org/news/bevy-0-15/) · [0.16](https://bevy.org/news/bevy-0-16/) · [0.17](https://bevy.org/news/bevy-0-17/) · [0.18](https://bevy.org/news/bevy-0-18/) · [0.19](https://bevy.org/news/bevy-0-19/)
- [Meshlet BVH Culling — PR #19318](https://github.com/bevyengine/bevy/pull/19318)
- [METIS-based meshlet generation — PR #16947](https://github.com/bevyengine/bevy/pull/16947)
- Virtual Geometry write-ups: [0.14](https://jms55.github.io/posts/2024-06-09-virtual-geometry-bevy-0-14/) · [0.15](https://jms55.github.io/posts/2024-11-14-virtual-geometry-bevy-0-15/) · [0.16](https://jms55.github.io/posts/2025-03-27-virtual-geometry-bevy-0-16/)
- Licence: Bevy is Apache-2.0. Designs are read and reimplemented; no file is transplanted, so no notice travels with it.
