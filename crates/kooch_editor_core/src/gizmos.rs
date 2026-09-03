//! Editor gizmo system — populates [`GizmoBatch`] from selection state.
//!
//! Architecture: each registered [`Visualizer`] in the
//! [`VisualizerRegistry`] is invoked once per selected entity that has
//! the corresponding component. Built-in visualizers (in the
//! [`visualizers`] submodule) cover Transform, cameras (perspective +
//! orthographic), and directional lights; user-extensibility lands
//! with `kooch_editor_api` (phase 4 of #278).
//!
//! Visibility is decided by [`GizmoVisibility`] and nothing else: every
//! registered visualizer runs for every selected entity, unless its
//! component or its category is switched off in the Gizmos panel.
//!
//! An entity can also be **pinned**, from the World panel's context
//! menu, and then its gizmos draw while something else is selected. That
//! is per entity rather than per component type on purpose: "show every
//! gravity field" is a real question, but the common one is "keep an eye
//! on this camera while I move what it follows", and answering it by
//! type floods the viewport with every other camera.
//!
//! Two earlier rules are gone. Gating on the Inspector's
//! `CollapsingHeader` (#581) coupled display to unrelated UI state.
//! Suppressing everything but `Transform` on a multi-selection (#587) was
//! a reasonable trade while there was no way to hide gizmos — but
//! comparing two colliders is a common thing to want, and it was exactly
//! the case that got suppressed. The panel is the escape hatch now.
//!
//! Transform *handles* remain single-selection: `HandleSet` positions one
//! origin, and multi-entity dragging needs pivot semantics of its own.

mod center_of_mass;
mod character;
mod collider;
mod facing;
mod gravity;
mod grounded;
#[cfg(test)]
pub(crate) mod harness;
mod physics_debug;

pub(crate) use physics_debug::PhysicsDebugOverlay;
mod lights;
mod parent_space;
mod touching;
mod virtual_camera;
mod visibility;
mod visualizers;
mod walk;

use std::any::TypeId;

use glam::{Mat3, Vec3};
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::{GlobalTransform, transform_propagation_system};
use kooch_ecs::orthographic_camera::OrthographicCamera;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::query::Query;
use kooch_ecs::transform::Transform;
use kooch_gizmos::{GizmoBatch, Gizmos, MeshBatch, VisualizerRegistry};
use kooch_gizmos_handles::{
    DragModifiers, HandleMode, HandleSet, Ray, SnapSettings, TransformDelta,
};

use crate::actions::EditorAction;
use crate::editor_camera::input::{HandleModeRequest, ViewportInputDelta};
use crate::state::{EditorOverlay, RotationDisplayMode};

pub(crate) use visibility::{
    GizmoGroup, GizmoVisibility, draw_gizmo_menu, groups_from_resources, load_visibility_system,
    save_visibility_system,
};
use visualizers::{OrthographicCameraVisualizer, PerspectiveCameraVisualizer};

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Inserts the `VisualizerRegistry` and registers built-in visualizers.
/// Also inserts a default `HandleSet` (3 translate handles X/Y/Z).
/// Runs once at editor startup.
pub(crate) fn register_builtin_visualizers_system(resources: &mut Resources) {
    let mut registry = resources.remove::<VisualizerRegistry>().unwrap_or_default();
    registry.register::<PerspectiveCamera, PerspectiveCameraVisualizer>();
    registry.register::<OrthographicCamera, OrthographicCameraVisualizer>();
    // A vcam has no mesh and no frustum of its own: without this it is an
    // empty entity in a list, and which way a framing aims is the whole
    // thing you are authoring.
    registry.register::<kooch_camera::VirtualCamera, virtual_camera::VirtualCameraVisualizer>();
    // Lights: where they point and how far they reach. `range` and the
    // cone angles are otherwise numbers with nothing to check them against.
    registry.register::<DirectionalLight, lights::DirectionalLightVisualizer>();
    registry.register::<kooch_ecs::point_light::PointLight, lights::PointLightVisualizer>();
    registry.register::<kooch_ecs::spot_light::SpotLight, lights::SpotLightVisualizer>();
    // A collider is authored as numbers and is otherwise invisible; the
    // outline is the only way to see whether the shape wraps the model.
    registry.register::<kooch_physics::components::Collider, collider::ColliderVisualizer>();
    // Where the author put the centre of mass. Only the authored one —
    // the solver's own is in the project's process, which is #634.
    registry
        .register::<kooch_physics::components::PhysicsBody, center_of_mass::CenterOfMassVisualizer>(
        );
    // A gravity field has no mesh, no surface and no contact: every number
    // it carries is world geometry that nothing else draws, and a rotated
    // zone is indistinguishable from an unrotated one until something falls
    // sideways.
    registry.register::<kooch_gravity::GlobalGravity, gravity::GlobalGravityVisualizer>();
    registry.register::<kooch_gravity::PointGravity, gravity::PointGravityVisualizer>();
    registry.register::<kooch_gravity::AreaGravity, gravity::AreaGravityVisualizer>();
    registry.register::<kooch_gravity::BoxGravity, gravity::BoxGravityVisualizer>();
    registry.register::<kooch_gravity::PlaneGravity, gravity::PlaneGravityVisualizer>();

    // A controller has no surface of its own: the ride height is a gap
    // that is supposed to be empty and the probe leaves no trace.
    registry.register::<kooch_character::CharacterController, character::CharacterVisualizer>();
    // What it was asking for, and what it found. The pair is the debug
    // view: a gap that does not match the ride height is visible rather
    // than deduced.
    registry.register::<kooch_character::Grounded, grounded::GroundedVisualizer>();
    // And what it was asked to do. A character that will not turn is two
    // arrows that disagree; one that turns the wrong way is two that
    // agree about the wrong thing.
    registry.register::<kooch_character::Facing, facing::FacingVisualizer>();
    // And what it decided to do about it. A character that will not stop
    // and one that is merely slow look the same standing still; the
    // difference is whether the goal went to zero.
    registry.register::<kooch_character::Walk, walk::WalkVisualizer>();
    // The wall, drawn like the ground: a slide that refuses to start is
    // either a wall nobody found or a normal pointing somewhere
    // unexpected, and those look identical in the Inspector.
    registry.register::<kooch_character::Touching, touching::TouchingVisualizer>();
    resources.insert(registry);

    if resources.get::<HandleSet>().is_none() {
        resources.insert(HandleSet::default());
    }
}

/// Pre-render system that rebuilds the gizmo line + mesh batches from
/// current selection by dispatching through the [`VisualizerRegistry`].
///
/// Timed as a whole (#691): it runs before the render system, so its
/// cost is real per-frame editor work that `cpu_frame_ms` does not
/// cover, and the physics overlay inside it walks the entire world
/// rather than the selection.
pub(crate) fn build_gizmo_batch_system(resources: &mut Resources) {
    let start = std::time::Instant::now();
    build_gizmo_batch(resources);
    crate::perf::record_gizmo_batch_ms(resources, crate::perf::ms_since(start));
}

fn build_gizmo_batch(resources: &mut Resources) {
    let (selected, pinned) = match resources.get::<EditorOverlay>() {
        Some(overlay) => (
            overlay.selected_entities.clone(),
            overlay.pinned_gizmos.clone(),
        ),
        None => return,
    };
    // Pinned entities draw alongside the selection, minus any that are
    // both — dispatching twice would double every line and make one
    // gizmo read brighter than its neighbours for no reason anyone
    // could act on.
    let also_drawn: Vec<Entity> = pinned
        .iter()
        .copied()
        .filter(|entity| !selected.contains(entity))
        .collect();

    let mut line_batch = resources.remove::<GizmoBatch>().unwrap_or_default();
    let mut mesh_batch = resources.remove::<MeshBatch>().unwrap_or_default();
    line_batch.clear();
    mesh_batch.clear();

    // Before the selection gate on purpose: the solver overlay describes
    // the whole world, and the questions it answers — what is touching
    // what, which bodies went to sleep — are asked precisely when nothing
    // is selected.
    physics_debug::draw(resources, &mut line_batch);

    if selected.is_empty() && also_drawn.is_empty() {
        resources.insert(line_batch);
        resources.insert(mesh_batch);
        return;
    }

    // What draws is an explicit choice now, not a side effect of which
    // Inspector header happens to be open. Absent resource = everything
    // visible, so a host that never inserted one behaves as before.
    let visibility = resources
        .get::<GizmoVisibility>()
        .cloned()
        .unwrap_or_else(GizmoVisibility::new);
    if !visibility.enabled {
        resources.insert(line_batch);
        resources.insert(mesh_batch);
        return;
    }
    // Resolved up front: the dispatch loop borrows Resources immutably,
    // and the category lives in the ComponentRegistry.
    let drawable: Vec<TypeId> = {
        let registry = resources.get::<VisualizerRegistry>();
        let components = resources.get::<ComponentRegistry>();
        registry
            .map(|r| {
                r.registered_types()
                    .filter(|type_id| {
                        let Some(components) = components.as_ref() else {
                            return true;
                        };
                        let Some(name) = components.component_name(type_id) else {
                            return true;
                        };
                        visibility.draws(name, components.reflect_category(type_id))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let registry = resources.remove::<VisualizerRegistry>().unwrap_or_default();

    {
        let mut gizmos = Gizmos::new(&mut line_batch, &mut mesh_batch);
        let resources_ref: &Resources = &*resources;
        for entity in selected.iter().chain(also_drawn.iter()) {
            for &type_id in &drawable {
                registry.dispatch(type_id, *entity, resources_ref, &mut gizmos);
            }
        }
    }

    // Pass 5: handles. They draw on top of the visualizers, with hover/
    // drag state managed by `HandleSet`. The handle set is populated
    // (origin updated) only when exactly one entity is selected — multi
    // and empty selections suppress translate handles for v1.
    if let Some(handle_set) = resources.get::<HandleSet>() {
        let mut gizmos = Gizmos::new(&mut line_batch, &mut mesh_batch);
        handle_set.draw(&mut gizmos);
    }

    resources.insert(line_batch);
    resources.insert(mesh_batch);
    resources.insert(registry);
}

/// Updates `HandleSet` from this frame's viewport input and applies any
/// resulting translation delta to the (single) selected entity. Runs
/// inside `editor_render_system` between input capture and camera
/// input apply, so the handle can absorb input before the camera
/// controller sees it.
///
/// `rotation_mode` must come from the caller — at the point where the
/// editor render system invokes this, `EditorOverlay` has already been
/// removed from `resources`, so reading it back here would always
/// fall back to `Local`.
///
/// Returns `true` when the handle is hovered or dragging — the caller
/// should skip applying camera-controller input on those frames.
///
/// `drag_start` is the persisted snapshot of the entity's `Transform`
/// at the moment the drag began. Cleared when the drag ends; used to
/// emit a single `EditorAction::TransformEdit` with before/after state
/// so the undo stack records one entry per drag (not per frame).
pub(crate) fn apply_handle_input(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    selected: &[Entity],
    rotation_mode: RotationDisplayMode,
    snap: SnapSettings,
    drag_start: &mut Option<(Entity, Transform)>,
    actions: &mut Vec<EditorAction>,
) -> bool {
    // Apply W / E / R mode request even when nothing is selected.
    if let Some(req) = delta.mode_request
        && let Some(handle_set) = resources.get_mut::<HandleSet>()
    {
        handle_set.set_mode(handle_mode_for_request(req));
    }

    // Single-entity v1: handles are suppressed for empty / multi selection.
    let target = match selected {
        [e] => *e,
        _ => {
            // Reset state so leftover hover/drag clears when selection changes.
            if let Some(handle_set) = resources.get_mut::<HandleSet>() {
                let _ = handle_set.update(
                    None,
                    false,
                    false,
                    DragModifiers::default(),
                    SnapSettings::default(),
                );
            }
            // Clear any stale snapshot — selection changed mid-drag.
            *drag_start = None;
            return false;
        }
    };

    let target_origin = match entity_world_position(resources, target) {
        Some(p) => p,
        None => return false,
    };

    let entity_rotation = entity_world_rotation(resources, target);
    let basis = match rotation_mode {
        RotationDisplayMode::Local => entity_rotation,
        RotationDisplayMode::World => Mat3::IDENTITY,
    };

    let ray = build_world_ray(resources, delta);

    let mut handle_set = match resources.remove::<HandleSet>() {
        Some(h) => h,
        None => return false,
    };
    handle_set.set_origin(target_origin);
    handle_set.set_basis(basis);
    handle_set.set_entity_rotation(entity_rotation);
    let modifiers = DragModifiers {
        ctrl: delta.ctrl_held,
        shift: delta.shift_held,
        alt: delta.alt_held,
    };
    let was_dragging = drag_start.is_some();
    let mode = handle_set.mode();
    let delta_out = handle_set.update(ray, delta.lmb_pressed, delta.lmb_held, modifiers, snap);
    let active = handle_set.is_active();
    let dragging = handle_set.is_dragging();
    resources.insert(handle_set);

    // Drag start: snapshot the entity's Transform BEFORE any mutation.
    // `HandleSet::update` returns `TransformDelta::none()` on the
    // Idle→Drag transition frame, so the snapshot is taken pre-mutation.
    if !was_dragging && dragging {
        if let Some(t) = read_transform(resources, target) {
            *drag_start = Some((target, t));
        }
    }

    // Apply the per-frame delta to the entity's local Transform.
    // `transform_propagation_system` re-derives the world matrix
    // downstream so the same-frame render sees the new pose.
    if dragging && !delta_out.is_noop() {
        // The gizmo is drawn from `GlobalTransform`, so its delta is in
        // world space; a `Transform` is in its parent's. For a root the
        // two coincide, which is why this went unnoticed until something
        // was parented — a child of a rotated parent slid down an axis
        // the user had not grabbed. See #612.
        let to_parent_space = parent_space::parent_world_to_local(resources, target);

        let mut mutated = false;
        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<Transform>()
            && let Some(t) = storage.get_mut(target)
        {
            match delta_out {
                TransformDelta::Translation(v) => {
                    let v = match to_parent_space {
                        Some(m) => parent_space::translation_to_parent_space(m, v),
                        None => v,
                    };
                    t.position += v;
                    mutated = true;
                }
                TransformDelta::Rotation(q) => {
                    let q = match to_parent_space {
                        Some(m) => parent_space::rotation_to_parent_space(m, q),
                        None => q,
                    };
                    // Left-multiply: the delta accumulates on the existing
                    // local rotation, both now in the parent's space.
                    t.rotation = q * t.rotation;
                    t.rotation = t.rotation.normalize();
                    mutated = true;
                }
                TransformDelta::Scale(factor) => {
                    // Component-wise multiply preserves any pre-existing
                    // non-uniform scale.
                    t.scale *= factor;
                    // Clamp to a tiny positive minimum so a fast drag
                    // can't collapse the entity to zero (which becomes
                    // un-recoverable since 0 × anything = 0).
                    t.scale = t.scale.max(Vec3::splat(0.001));
                    mutated = true;
                }
            }
        }
        if mutated {
            transform_propagation_system(resources);
        }
    }

    // Drag end: emit one TransformEdit action with before/after.
    // Compares against the snapshot to skip no-op drags (clicked the
    // handle but didn't move).
    if was_dragging
        && !dragging
        && let Some((entity, before)) = drag_start.take()
        && let Some(after) = read_transform(resources, entity)
        && !transforms_equal(before, after)
    {
        actions.push(EditorAction::TransformEdit {
            entity,
            before,
            after,
            desc: handle_mode_desc(mode),
        });
    }

    active
}

fn read_transform(resources: &Resources, entity: Entity) -> Option<Transform> {
    resources
        .get::<ComponentRegistry>()
        .and_then(|reg| reg.get_cpu::<Transform>())
        .and_then(|storage| storage.get(entity))
        .copied()
}

fn transforms_equal(a: Transform, b: Transform) -> bool {
    a.position == b.position && a.rotation == b.rotation && a.scale == b.scale
}

fn handle_mode_desc(mode: HandleMode) -> &'static str {
    match mode {
        HandleMode::Translate => "Move Entity",
        HandleMode::Rotate => "Rotate Entity",
        HandleMode::Scale => "Scale Entity",
    }
}

/// Constructs a world-space ray from the viewport cursor + active
/// camera. Returns `None` when the cursor isn't over the viewport or
/// no active perspective camera exists.
///
/// The unprojection itself lives in `kooch_render::projection`, beside the
/// reversed-Z projection it inverts — a second copy here is a second place
/// to forget that the far plane is `ndc.z = 0` (#488). This function is now
/// only the part that is about *this* editor: finding the active camera and
/// the cursor.
fn build_world_ray(resources: &Resources, delta: ViewportInputDelta) -> Option<Ray> {
    let cursor = delta.cursor_local?;
    let (camera, gt) = active_camera(resources)?;

    let ray = kooch_render::projection::viewport_cursor_to_ray(
        cursor,
        delta.viewport_size,
        gt.matrix,
        camera.fov.to_radians(),
        camera.near,
    )?;
    Some(Ray::new(ray.origin, ray.direction))
}

/// The highest-priority perspective camera in the world.
///
/// `pub(crate)` because a viewport drop has to unproject against the same
/// camera the gizmos pick with — two answers to "which camera" would let a
/// handle and a drop disagree about where the cursor points.
pub(crate) fn active_camera(resources: &Resources) -> Option<(PerspectiveCamera, GlobalTransform)> {
    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut best: Option<(i32, PerspectiveCamera, GlobalTransform)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active {
            return;
        }
        let better = match &best {
            Some((p, _, _)) => cam.priority > *p,
            None => true,
        };
        if better {
            best = Some((cam.priority, *cam, *gt));
        }
    });
    drop(query);
    best.map(|(_, c, g)| (c, g))
}

fn handle_mode_for_request(req: HandleModeRequest) -> HandleMode {
    match req {
        HandleModeRequest::Translate => HandleMode::Translate,
        HandleModeRequest::Rotate => HandleMode::Rotate,
        HandleModeRequest::Scale => HandleMode::Scale,
    }
}

fn entity_world_position(resources: &Resources, entity: Entity) -> Option<Vec3> {
    let registry = resources.get::<ComponentRegistry>()?;
    let storage = registry.get_cpu::<GlobalTransform>()?;
    let gt = storage.get(entity)?;
    Some(gt.matrix.w_axis.truncate())
}

/// Reads the entity's world-space rotation from `GlobalTransform`.
/// Used as both the Local-mode display basis and the
/// `entity_world_rotation` always-on field used by `ScaleHandle` to
/// convert World-space drag intent into local-space scale factors.
///
/// `to_scale_rotation_translation` is lossy under shear; for our
/// typical scene hierarchies that's acceptable. See PR #217 / the
/// shear decision in the Decisions Log.
fn entity_world_rotation(resources: &Resources, entity: Entity) -> Mat3 {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Mat3::IDENTITY;
    };
    let Some(storage) = registry.get_cpu::<GlobalTransform>() else {
        return Mat3::IDENTITY;
    };
    let Some(gt) = storage.get(entity) else {
        return Mat3::IDENTITY;
    };
    let (_, rotation, _) = gt.matrix.to_scale_rotation_translation();
    Mat3::from_quat(rotation)
}
