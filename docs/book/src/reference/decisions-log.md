# Decisions Log

Chronological record of architectural decisions. Each entry captures
**what** was decided, **why**, and **what it cost** (or commits us to).
Reading this is faster than reading 12 PR descriptions to figure out why
a function exists.

Format:

> **YYYY-MM-DD · Title** *(refs)*
>
> **Decision:** one-paragraph summary.
> **Why:** the constraint or insight that drove it.
> **Consequence:** what this commits the project to, or what it rules out.

---

## Coordinate system

> **Permanent · Right-handed, -Z forward, Y up**
>
> **Decision:** Use the same convention as glTF, OpenGL, Vulkan, Blender,
> Maya, and `glam`'s default view/projection matrices. -Z is forward, +Y
> is up, +X is right.
> **Why:** Going against the grain means flipping Z at every loader
> boundary (glTF importer, USD, FBX, exported camera transforms). Unity
> picked left-handed because of DirectX heritage; they pay the flip cost
> in every importer. We don't want to.
> **Consequence:** Identity quaternion `(0,0,0,1)` faces -Z. Anyone
> coming from Unity (left-handed +Z forward) needs to mentally flip when
> authoring scenes.

---

## SDF ray-marching as primary render path

> **Permanent · Sphere tracing, not rasterization**
>
> **Decision:** Primary render pipeline is SDF sphere tracing. Mesh
> rasterization is a secondary pass layered on top.
> **Why:** Want experimentation latitude (Mario Galaxy gravity, infinite
> procedural geometry, smooth blends) that rasterization can't give
> cheaply. SDFs are also a clean GPU-resident data model that pairs well
> with the hybrid ECS.
> **Consequence:** Performance ceiling is lower than a modern PBR
> rasterizer at the same hardware budget. Hybrid mesh+SDF (Dreams style)
> is the long-term escape hatch but ~3000 LOC away.

---

## Hierarchical coordinate scales (NOT floating origin)

> **2026-04 · Issue #50** *(blocks #51, #52, #54, #90)*
>
> **Decision:** Universe (i64 sector + f64 offset) → Solar system (f64) →
> Planet (f32) → Surface (f32) → Camera-relative render (f32). Origin
> rebasing is a *trigger* when the player gets far from origin, not the
> sole mechanism.
> **Why:** Inspired by No Man's Sky, Star Citizen, KSP. Pure floating
> origin works for one scale (Outer Wilds) but breaks down across
> astronomical ↔ surface transitions.
> **Consequence:** Multiple `Transform` types, conversion at scale
> boundaries, but precision stays bounded inside each scale. Locks in
> design for camera-relative transforms (#51), sector boundaries (#52),
> world streaming (#54), and navigation (#90).

---

## SDF tracer roadmap

> **2026-04-22 · Step count bump 128 → 256** *(PR #227 closes #221)*
>
> **Decision:** Quick fix raised `max_steps` default from 128 to 256.
> Visible gaps in concave SDF necks reduced to imperceptible.
> **Why:** Two attempts at smarter tracers failed:
> Enhanced Sphere Tracing (PR #222, branch killed) and the iq closed-form
> ellipsoid (#229, killed) both produced gray patches at CSG seams.
> Brute force was the surgically minimal change that worked.
> **Consequence:** ~2× iteration cost on the GPU, predictable. Locked
> until Segment Tracing (Galin et al. 2020, issue #224) lands and lets
> us drop back to ~128 steps with correct Lipschitz bounds.

> **2026-04-22 · ESL is dead** *(#222 killed twice)*
>
> **Decision:** Do not retry Enhanced Sphere Tracing while the shader
> uses the `s_min` workaround for non-uniform scale. Two attempts on
> 2026-04-22 confirmed it cannot work. Project memory documents the dead
> ends.
> **Why:** Naive ESL with non-Lipschitz CSG produces poly-edged gray
> patches near silhouettes (calc_normal crosses gradient discontinuities
> at seams). Lowering omega from 1.6 to 1.3 makes it *worse*, not
> better. Hybrid (over-relax only in far field) doesn't fix it either.
> **Consequence:** Segment Tracing (#224) is the only open path forward
> for tracer optimization.

---

## Editor camera as ephemeral ECS entity

> **2026-04-22 · `EditorCamera + EditorOnly + PerspectiveCamera + Transform`** *(#199 → PR #219)*
>
> **Decision:** The editor camera is a regular ECS entity, not a
> `Resource`. It carries an `EditorOnly` marker that the
> `EphemeralComponents` filter checks during scene serialization — the
> entity is invisible to `from_ecs` and to `despawn_all`.
> **Why:** The renderers already iterate cameras by priority. Resource
> approach would require a special code path in every renderer and a
> manual mode swap. Entity approach lets future editor-only entities
> (gizmos, grid, debug lights) ride the same `EditorOnly` filter without
> new plumbing.
> **Consequence:** Play mode strips the editor camera. Scenes need their
> own non-ephemeral active `PerspectiveCamera` to render anything in
> play. UX feature "Play uses editor view" is a separate future issue.

> **2026-04-22 · Quat internally for cameras, Euler-cached for inspector**
>
> **Decision:** Cameras store `Quat` internally for orbit/fly rotations.
> The inspector caches Euler angles per-field to avoid gimbal lock and
> to keep each X/Y/Z field stable while the others are edited.
> **Why:** Inspector UX needs each axis editable independently of the
> others — a `Quat` round-trip mangles two axes when you edit the third.
> Cameras need continuous quaternion math for cinematics. Different
> contexts, both correct, do not unify.
> **Consequence:** Two rotation representations exist in the codebase.
> Documented; do not "simplify".

> **2026-04-22 · Fly-mode pivot is camera position, NOT focus point** *(#199 PR #219)*
>
> **Decision:** In orbit mode, rotation pivots around `focus_point`. In
> fly mode, it pivots around the camera itself; `focus_point` is
> re-anchored to `camera_position + forward * distance` after each
> rotation.
> **Why:** Bug found in manual testing — without this invariant, fly
> mode would drift laterally as you looked around (your "feet" moved
> when you turned your head).
> **Consequence:** `EditorCameraController` carries explicit logic for
> the two modes; do not refactor toward a unified pivot.

---

## Render orchestration

> **2026-04-23 · Editor is the render orchestrator for offscreen** *(PR #235 closes #129)*
>
> **Decision:** `kooch_editor_core::systems::startup` instantiates
> `RayMarchRenderer + MeshPassRenderer + SkyRenderPass` directly as
> `Resources` and `viewport::render::render_viewport` runs the three
> passes in one encoder against the offscreen `ViewportTarget`. The
> `RayMarchPlugin` is **not** used by the editor.
> **Why:** Doing this through a `RenderGraph` abstraction would have
> been ~500 LOC for a 3-pass pipeline. Plain procedural orchestration
> wins until there are 5+ passes.
> **Consequence:** When a fourth pass (post-process composite, G-Buffer,
> shadow map) is added, re-evaluate building a `RenderGraph`.

> **2026-04-25 · `RenderPlugin` IS the game render path** *(PR #267 closes #260)*
>
> **Decision:** `RenderPlugin` (in `kooch_render`) is the play-binary
> orchestrator. Same 3-pass pipeline as the editor's `render_viewport`,
> but writing to the swapchain surface instead of an offscreen texture.
> `RayMarchPlugin` stays as the standalone demo path (`raymarch_demo`).
> **Why:** Stub `RenderPlugin` that only cleared the screen was dead
> weight. The semantically right name for "the game's render plugin" is
> `RenderPlugin`. No separate `GameRenderPlugin` invented.
> **Consequence:** Editor and play share one conceptual model with two
> orchestration callsites. A future regression in either path is
> immediately reproducible in the other.

> **2026-04-23 · Mesh pass: two pipelines, one target, one encoder** *(#129)*
>
> **Decision:** Raymarch pipeline runs first with `LoadOp::Clear`, mesh
> pipeline runs second on the same target with `LoadOp::Load`. No shared
> shader, no unified material system — explicitly NOT unified.
> **Why:** Unifying the SDF shader and the mesh shader would have been a
> multi-week refactor for an MVP feature. Two pipelines is correct
> enough.
> **Consequence:** Material system per-pipeline grows independently
> until #130 PBR forces convergence.

> **2026-04-23 · `Depth32Float` constant + `LessEqual` for sky** *(PR #237 closes #236)*
>
> **Decision:** All passes share `VIEWPORT_DEPTH_FORMAT = Depth32Float`
> as a public `kooch_render` constant. Sky pipeline uses
> `CompareFunction::LessEqual`; mesh pipeline uses `Less`.
> **Why:** Sky writes `frag_depth = 1.0` explicitly so meshes behind it
> can supersede. Depth clears to 1.0. With `Less`, `1.0 < 1.0` is false
> and sky never draws — black viewport. Bug found in first manual test.
> Mesh keeps `Less` because no mesh is exactly at the far plane.
> **Consequence:** Future depth format change is one line in
> `lib.rs`. Documented in shader comments next to the comparison choice.

---

## Sky / atmosphere

> **2026-04-23 · `SkyRenderer` does NOT blend between multiple skies** *(PR #247 closes #246)*
>
> **Decision:** `SkyRenderer` is a singleton-by-priority component.
> Highest-priority active wins; no crossfade composite pass. Day/night
> is animated within one shader, not by blending two materials.
> **Why:** Crossfading entire sky materials was scope creep. Unity,
> Unreal, and Bevy don't do it natively either. Animated parameters
> within one material handle the real-world use case.
> **Consequence:** No `SkyComposite` pass. If we ever need sky
> crossfade, that's a new pass with explicit cost.

> **2026-04-23 · `SkyRenderer` and `AtmosphereVolume` are separate components**
>
> **Decision:** `SkyRenderer` = singleton ambient backdrop (deep space
> or default gradient). `AtmosphereVolume` = volumetric shell per-planet
> with scattering, N coexisting in the world.
> **Why:** Architecture of `stellar_delivery` and Unreal's
> `SkyAtmosphere`. Singleton sky and per-planet atmosphere have
> different lifetimes, coordinate frames, and shader budgets. Forcing
> one component to do both invents complexity.
> **Consequence:** Two paths to maintain, both simpler than one
> overloaded path. `AtmosphereVolume` ships in a future PR (#248).

---

## Scene management

> **2026-04-24 · `SceneManager` agnostic of component types** *(PR #266 closes #259)*
>
> **Decision:** `SceneManager` lives in `kooch_ecs` and knows nothing
> about Camera, Sky, or any specific component. The default scene
> bootstrap (Camera + Sky entities written to disk on project create)
> lives in `kooch_editor_core::project::ensure_default_scene`.
> **Why:** Same split as `EphemeralComponents`: mechanism in core,
> policy in editor. Lets `SceneManager` be reused by headless tools that
> have a different "default scene" idea.
> **Consequence:** `kooch_ecs` cannot be the place to teach the engine
> "every project starts with a Camera and a Sky." That decision is the
> editor's.

> **2026-04-25 · Scene bootstrap runs at `Stage::First`, NOT `Stage::Startup`** *(PR #267 closes #260)*
>
> **Decision:** `SceneBootstrapPlugin::load_boot_scene` runs at
> `Stage::First`, which fires once-per-frame *after* all
> `Stage::Startup` systems complete. The `BootScene` resource is
> consumed on first call so it's effectively a one-shot.
> **Why:** Race detected in manual testing — if user
> `register_components` and SceneBootstrap both ran at `Stage::Startup`,
> scene deserialization happened before `Player` (custom component)
> registered → `unknown component type: Player` error. `Stage::First`
> guarantees a clean handshake.
> **Consequence:** Replicable pattern for any future plugin that
> depends on user-registered state. First frame waits one stage tick
> for the scene to appear; imperceptible at 60 FPS.

> **2026-04-25 · Play uses `cargo run --manifest-path`, no exe-detection** *(PR #267 closes #260)*
>
> **Decision:** `EditorAction::Play` runs
> `cargo run --manifest-path <project>/Cargo.toml -- --scene <abs>`.
> The old `is_project_binary` flag and `current_exe.starts_with(target)`
> guard are gone.
> **Why:** Cargo handles incremental build, caching, and run as one
> primitive. Custom exe detection only worked for the half of project
> launches that ran the binary directly; not for in-process
> `OpenProject` flows. The new approach works for both.
> **Consequence:** First Play after a code change costs a `cargo build`
> (~0.1–30s). Editor stays responsive (cargo runs as child).
> Async-build modal with cancel is a future UX issue, not architecture.

> **2026-04-25 · Project template is play-mode-only** *(PR #267 closes #260)*
>
> **Decision:** Generated `main.rs` is ~10 lines:
> `App::new() + DefaultPlugins + register_components`. The dual
> editor/play branching the old template carried is gone — the editor
> is its own binary, never embedded in user crates.
> **Why:** Cleaner mental model, cleaner code. The "editor inside the
> user binary" pattern was a leftover from before the editor binary
> existed; it confused exe-detection and confused users.
> **Consequence:** Existing user projects need migration (one-line
> change in `main.rs` + Cargo.toml cleanup). New projects are clean.

---

## wgpu strategy

> **2026-04-23 · Stay on wgpu 29 for 24 months minimum** *(PR #239 closes #238)*
>
> **Decision:** Do not migrate to ash / vulkano / dx12-rs. No active
> migration trigger. Hybrid wgpu + ash only if RT pipelines become a
> requirement *and* upstream issue `#8560` (Metal pipelines design)
> stays unresolved past April 2028.
> **Why:** Bevy ships Solari (path-traced GI) on wgpu in September
> 2025. If they don't migrate prematurely, we don't either. The audit
> in `docs/research/wgpu-capabilities.md` lists 5 concrete migration
> triggers; none are active.
> **Consequence:** No raw Vulkan / Metal escape hatch in user code.
> Specific blocked features (mesh shaders cross-backend, FSR 2 viable,
> 3D texture arrays, GPU memory reporting) work around or wait.

> **2026-04-24 · `PipelineCache` with `fallback: true`** *(PR #257 closes #251)*
>
> **Decision:** Enable `wgpu::PipelineCache` keyed on
> `(adapter.name, driver_info, engine_version)`. Save on `Drop for
> GpuContext`; SIGKILL is tolerated.
> **Why:** 100–500 ms cold-start saving per pipeline. The `unsafe` of
> `Device::create_pipeline_cache` is covered by `fallback: true` —
> driver rejects an invalid blob without UB. Hash key invalidates
> on driver upgrades.
> **Consequence:** `~/.cache/ome/pipeline_cache/<hash>.bin` files
> accumulate (they're tiny). Deleting them is harmless; engine
> regenerates on next run.

> **2026-04-24 · `PowerProfile` enum lives in `kooch_core::power`** *(PR #258 closes #253)*
>
> **Decision:** `PowerProfile::{Plugged, Balanced, Battery, Debug}` as a
> `Resource` in `kooch_core::power`. Auto-detect on Linux via sysfs and
> `$SteamDeck` env var. Override via `KOOCH_POWER_PROFILE`.
> **Why:** The Steam Deck / OneXFly target makes battery awareness
> non-negotiable. Renderers will gate quality defaults (DoF, SSR, TAA
> off in Battery) per-feature in future PRs.
> **Consequence:** `kooch_core` carries the policy enum but renderers do
> not yet read it. Integration is per-feature PR work, intentional.

---

## Inspector / editor UX

> **2026-04 · `GlobalTransform` tolerates shear, inspector warns** *(PR #217 closes #214)*
>
> **Decision:** `GlobalTransform` is a 4×4 matrix that can carry shear
> (non-uniform scale through a rotated parent), but the inspector does
> *not* attempt to decompose it. Instead it shows a warning icon and
> exposes a `lossy_scale()` helper.
> **Why:** Decomposing shear is ambiguous (multiple TRS triplets
> reproduce the same matrix). Hiding the issue creates worse bugs
> downstream. Educating the user is honest.
> **Consequence:** Users authoring shear-causing parent chains see the
> warning. No automatic "fix" is offered.

> **2026-04-25 · Three-system editor architecture: Gizmos / Editor / UI Toolkit** *(research [#276](https://github.com/lobinuxsoft/kooch/issues/276), doc `docs/research/editor-three-system-architecture.md`)*
>
> **Decision:** Editor evolves into three separate, pure-Rust,
> custom-built subsystems: **`kooch_gizmos`** (visual gizmo API +
> visualizer registry, usable at runtime too) + **`kooch_gizmos_handles`**
> (interactive translate/rotate/scale, editor-only); **`kooch_editor_api`**
> (user editor extensions: inspectors, panels, actions, loaded via
> libloading from a user `editor/` crate); **`kooch_ui`** (declarative
> HTML-like UI Toolkit: `.kooch_ui` markup + `.kooch_style` CSS subset +
> Rust behavior, retained-mode with fine-grained signals, coexists
> with `egui`).
> **Why:** Godot's self-hosted monolith couples concerns; Unity's
> separation (Gizmos / Handles / Editor scripts / UI Toolkit) lets
> each evolve independently and gives users one mental model per
> need. We follow Unity's separation. External libraries —
> `transform-gizmo`, Slint, Dioxus — rejected: only cover narrow
> slices, none address user-extensibility for custom component
> visualizers, and we want the engine to be self-contained pure
> Rust with no FFI.
> **Consequence:** A multi-quarter commitment. Three implementation
> epics (one per subsystem) replace the original gizmo epic #198 as
> sub-epic of the Gizmos one. The current `kooch_render::gizmos`
> module (PR #277) migrates into `kooch_gizmos` in phase 1. The
> `kooch_ui` toolkit is the heaviest piece (multi-month) and runs in
> parallel with the others.

> **2026-07-25 · Keep `kooch_ecs`; do not adopt `bevy_ecs`** *(decision [#605](https://github.com/lobinuxsoft/kooch/issues/605))*
>
> **Decision:** `kooch_ecs` stays and improves in place. `bevy_ecs` is the
> reference to steal individual designs from, never a dependency.
> **Why:** #603 removed the GPU component storages that had justified a
> custom ECS, so the justification was re-derived from measurements
> rather than repeated. No technical blocker was found — `bevy_ecs` is
> genuinely standalone (65 crates, no `bevy_app`/`bevy_render`), the
> GPU-driven renderer touches the ECS through `Query` in four places, and
> `bevy_reflect` expresses our custom field attributes. What decided it:
> 42 call sites reach into component storage directly against 3 that use
> `Query`, which is work required in *every* path and which `kooch_ecs`
> can already express; `EntityAllocator::revive` preserves entity
> identity across Play/Stop, which `bevy_ecs` refuses by design while 177
> sites outside the crate hold an `Entity` in a field; and 51 of the 80
> affected files are `kooch_editor_core`, the one area where Bevy offers no
> upstream design to copy because it has no editor.
> **Consequence:** improvements are ordered by demonstrated pain, not by
> feature parity. Encapsulating the ECS behind `Query` is the
> prerequisite for any future backend change — today the contact surface
> is 80 files. The schedule graph belongs in `kooch_core`, not the ECS:
> the ordering bugs it would fix live in `app.rs`.

> **2026-07-25 · Entities are referenced by a persistent id, not a handle or an index** *(feat [#607](https://github.com/lobinuxsoft/kooch/issues/607))*
>
> **Decision:** a component may hold an `Entity` and have it survive a
> save. Identity is an opt-in `PersistentId(EntityGuid)`; the wire form
> is `EntityRef`, which is `Live(Entity)` in memory and
> `Persistent { scene, id }` on disk. Ids are scene-local and remapped
> per instance. `Parent` becomes an ordinary component and
> `parent_index` is legacy-read-only.
> **Why:** reflection had no way to express "points at an entity", so the
> scene format carried the parent link out of band. That worked for one
> component and could not scale: joints hold two entities, and an index
> into one document cannot address another scene at all. Assets had
> already solved the same problem by addressing through a `Guid`.
> Scene-local ids follow Unity (`SceneLoadFlags.NewInstance`) and Unreal
> (Level Instances), and are what allows one scene to be instantiated
> twice without both copies claiming the same identity.
> **Consequence:** saving a scene mutates the world, because whether an
> entity is referenced is only known once references are written —
> `SceneDocument::from_ecs` takes `&mut Resources`. `Entity` still does
> not implement `Serialize`, so serialising a live reference is an error
> rather than a handle written to disk. A reference whose target is
> absent saves and loads as unset, which is the normal state for a
> reference into a non-resident cell under #566. Unblocks #560 and
> cross-scene references.

> **2026-07-25 · The world is the container; scenes are content loaded into it** *(feat [#609](https://github.com/lobinuxsoft/kooch/issues/609))*
>
> **Decision:** `SceneManager` becomes a registry of open scenes with one
> active, instead of a single current path whose load replaced the world.
> Scenes carry a `Guid`; `SceneMember` records an entity's authoring home
> and is derived on load rather than serialised. Saving writes only one
> scene's entities; closing despawns only its own.
> **Why:** the model #566 settled on. One scene per world is "the entire
> world in one section", which cannot express a space station and an
> asteroid field as separate content occupying the same volume, nor make
> "close the station" different from "walk away from it". #607 supplied
> the prerequisite by making entity references survive a save.
> **Consequence:** there is always a scene, even before the first save,
> and entities with no membership are adopted by the active scene when it
> saves — otherwise anything spawned in the editor would belong to nothing
> and be written to no file. The reference remap table is keyed by
> `(scene, id)`, never by id alone: ids are scene-local, so two open
> scenes both numbering an entity 1 is ordinary. Opening the same file
> twice is refused, because two copies would share every entity id.
> Scene transforms and instancing are deliberately deferred — they need a
> decision on whether the transform bakes at load, as Unreal's Embedded
> Level Instances do.

> **2026-07-26 · A scene is the prefab; instancing and editing are different operations** *(epic [#611](https://github.com/lobinuxsoft/kooch/issues/611))*
>
> **Decision:** prefabs are scenes instanced with their entity ids remapped
> per instance. No separate format. Built in two phases: runtime
> instancing first, the linked-with-overrides prefab system after.
> **Why:** #609 refuses to open one file twice, which is right for editing
> and wrong as a limit on instancing — and entity ids were made
> scene-local in #607 precisely so instances could remap them. Unity's
> prefab *is* a serialised scene file, and Godot says so outright with
> `PackedScene`; both store an instance as a reference to the source plus
> a list of differences rather than a copy, which is what keeps editing
> the source propagating to its instances.
> **Consequence:** two things must be settled in phase A because they
> touch already-merged types — whether a scene must have a single root
> (instancing as a unit with a transform needs one, and our documents are
> a flat list), and how an outside reference names *this instance* rather
> than the prefab, since `EntityRef::Persistent { scene, id }` is
> ambiguous once a scene is instanced twice. Phase B waits on one
> decision: whether overrides are per field, as Unity and Godot both do,
> or whether editing an instance promotes it to its own scene.
