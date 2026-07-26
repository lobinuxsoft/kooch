# Roadmap

What is next and why, ordered by what blocks what. Issue bodies hold the detail; this is the
map.

Companion to [`MEMORY.md`](MEMORY.md), which records decisions already made. If the two
disagree, `MEMORY.md` wins on *decisions* and this file wins on *order*.

Last updated 2026-07-26, `development` at `b2e4ddc`.

---

## Done recently

| | |
|---|---|
| **#605** | The custom ECS stays. `bevy_ecs` evaluated on measurements and declined |
| **#607** | Stable entity references — a component can point at an entity and survive a save |
| **#609** | More than one scene loaded at once |
| **#612** | Parented physics — compound colliders, gizmo parent-space conversion, Inspector warnings |

---

## Next — physics, because half of it is missing where users look

Smoke turned up that the physics story has a visible hole: the editor tells authors to use
a joint, and there are no joints.

1. **#560 — joints.** Unblocked by #607, and now the answer the Inspector already points at.
   Rapier offers fixed, revolute, prismatic, spherical, rope, spring, pin-slot and generic,
   in impulse and multibody flavours, with motors, limits and breaking. Shape: one `Joint`
   component with a reflected type discriminant and `FieldCondition` per-type fields — the
   pattern #586 established for `Collider`.
2. **#618 — mass control.** Compound bodies compute their centre of mass from every shape,
   which surprised the author and reads as "slow". The physics is right; the control is
   missing. Needs a decision on the default before code.
3. **#563 — physics debug render.** Would have made #618 diagnosable in seconds instead of
   by reasoning about inertia tensors. Cheap, and it pays for itself the next time physics
   looks wrong.

Then the rest of what Rapier exposes and the engine does not: **#561** (events, sensors,
groups), **#562** (scene queries), **#567** (PD/PID controllers), **#569** (per-stage
counters in the perf HUD).

> The standing rule: implement what Rapier offers, warn for what it does not. Everything
> above is exposing the solver, not working around it. See `MEMORY.md`.

---

## Next — editor, because multi-scene is half-reachable

1. **#619 — no way to create a scene.** There is no New Scene. Multi-scene cannot be
   exercised without files already on disk.
2. **#613 — additive scenes over the wire.** The protocol assumes one scene, so additive
   loading is disabled while a project is open. The only place a merged feature is
   knowingly incomplete.
3. **#591** context menus, **#592** a Game panel separate from View.

---

## Then — prefabs

**#611**, in two phases. Phase A is runtime instancing: a scene instanced with its entity
ids remapped. Phase B is the linked-with-overrides prefab system.

Two things phase A must settle because they touch merged types: whether a scene needs a
single root, and how an outside reference names *this instance* rather than the prefab —
`EntityRef::Persistent { scene, id }` is ambiguous once a scene is instanced twice.

---

## Larger, not yet started

- **#566 — world cells.** Scenes become streamable content; entities transit between cells.
  Scene and cell are orthogonal axes. This is the piece planet-scale actually needs.
- **#614 — terrain LOD.** Research, with an honest verdict: dual contouring beats
  marching-cubes-plus-Transvoxel on an octree, but feeding octree nodes through the meshlet
  pipeline is **an unproven hypothesis**. Nobody found doing it. Measure the cost of
  clusterising one dirty node before any design commits.
- **Rendering backlog** — #476/#477 shadows, #450 GI, #485 clustered light culling, #484
  HDR, #481 motion vectors and FSR. Unblocked, unscheduled.
- **#558** — shippable builds must exclude the editor. Security, not size.

---

## Deliberately not scheduled

- **Adopting Bevy wholesale.** Would save roughly two thirds of the codebase and cost the
  parts that are the point: the GPU-driven meshlet renderer, planet-scale streaming, and the
  editor — none of which Bevy provides. Settled in #605.
- **Making a dynamic body follow its parent.** No engine supports it; Godot has failed to
  for years. The supported answers are compound colliders (#615, done) and joints (#560).
