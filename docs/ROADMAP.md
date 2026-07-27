# Roadmap

What is next and why, ordered by what blocks what. Issue bodies hold the detail; this is the
map.

Companion to [`MEMORY.md`](MEMORY.md), which records decisions already made. If the two
disagree, `MEMORY.md` wins on *decisions* and this file wins on *order*.

Last updated 2026-07-26, `development` at `63df64c`.

---

## Done recently

| | |
|---|---|
| **#605** | The custom ECS stays. `bevy_ecs` evaluated on measurements and declined |
| **#607** | Stable entity references — a component can point at an entity and survive a save |
| **#609** | More than one scene loaded at once |
| **#612** | Parented physics — compound colliders, gizmo parent-space conversion, Inspector warnings |
| **#560** | Joints — one `Joint` component, all eight rapier kinds, motors, limits, breaking |
| **#618** | Mass control — `mass` means kilograms, shapes are massless, explicit centre of mass |
| **#563** | Physics debug render — the solver's own account of itself, in the viewport |
| **#623** | Collider material — friction, restitution, combine rules and damping, all authorable |
| **#561** | Collision events, sensors and groups — the solver can finally report back |

---

## Next — physics, because half of it is missing where users look

Smoke turned up five holes, and all five are closed: no joints where the editor said to use
one, a mass that came from nowhere, no way to look at any of it, a material nobody could
touch, and a solver that could not report back — and gravity that only pointed down. What is
left is the queries nobody can make.

**#624 is done** — `crates/ome_gravity/` is no longer a nine-line placeholder. Three source
components that sum, applied as an impulse so gameplay forces still compose.

Next: **#562** (scene queries), **#567** (PD/PID controllers), **#569** (per-stage counters in
the perf HUD). Loose end from #624: splitting `RigidBody` into `RigidBody`/`KinematicBody`/
`StaticBody`, which is a scene-format migration and wants its own issue.

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
