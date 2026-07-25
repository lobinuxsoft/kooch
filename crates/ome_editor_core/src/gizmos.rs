//! Editor gizmo system — populates [`GizmoBatch`] from selection state.
//!
//! Architecture: each registered [`Visualizer`] in the
//! [`VisualizerRegistry`] is invoked once per selected entity that has
//! the corresponding component. Built-in visualizers (in the
//! [`visualizers`] submodule) cover Transform, cameras (perspective +
//! orthographic), and directional lights; user-extensibility lands
//! with `ome_editor_api` (phase 4 of #278).
//!
//! Visibility is decided by [`GizmoVisibility`] and nothing else: every
//! registered visualizer runs for every selected entity, unless its
//! component or its category is switched off in the Gizmos panel.
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

mod collider;
mod visibility;
mod visualizers;

use std::any::TypeId;

use glam::{Mat3, Vec3, Vec4};
use ome_core::resource::Resources;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::directional_light::DirectionalLight;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::{GlobalTransform, transform_propagation_system};
use ome_ecs::orthographic_camera::OrthographicCamera;
use ome_ecs::perspective_camera::PerspectiveCamera;
use ome_ecs::query::Query;
use ome_ecs::transform::Transform;
use ome_gizmos::{GizmoBatch, Gizmos, MeshBatch, VisualizerRegistry};
use ome_gizmos_handles::{DragModifiers, HandleMode, HandleSet, Ray, SnapSettings, TransformDelta};

use crate::actions::EditorAction;
use crate::editor_camera::input::{HandleModeRequest, ViewportInputDelta};
use crate::state::{EditorOverlay, RotationDisplayMode};

pub(crate) use visibility::{
    GizmoGroup, GizmoVisibility, draw_gizmo_menu, groups_from_resources, load_visibility_system,
    save_visibility_system,
};
use visualizers::{
    DirectionalLightVisualizer, OrthographicCameraVisualizer, PerspectiveCameraVisualizer,
};

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
    registry.register::<DirectionalLight, DirectionalLightVisualizer>();
    // A collider is authored as numbers and is otherwise invisible; the
    // outline is the only way to see whether the shape wraps the model.
    registry.register::<ome_physics::components::Collider, collider::ColliderVisualizer>();
    resources.insert(registry);

    if resources.get::<HandleSet>().is_none() {
        resources.insert(HandleSet::default());
    }
}

/// Pre-render system that rebuilds the gizmo line + mesh batches from
/// current selection by dispatching through the [`VisualizerRegistry`].
pub(crate) fn build_gizmo_batch_system(resources: &mut Resources) {
    let selected = match resources.get::<EditorOverlay>() {
        Some(overlay) => overlay.selected_entities.clone(),
        None => return,
    };

    let mut line_batch = resources.remove::<GizmoBatch>().unwrap_or_default();
    let mut mesh_batch = resources.remove::<MeshBatch>().unwrap_or_default();
    line_batch.clear();
    mesh_batch.clear();

    if selected.is_empty() {
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
        for entity in &selected {
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
        let mut mutated = false;
        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<Transform>()
            && let Some(t) = storage.get_mut(target)
        {
            match delta_out {
                TransformDelta::Translation(v) => {
                    t.position += v;
                    mutated = true;
                }
                TransformDelta::Rotation(q) => {
                    // Left-multiply: world rotation accumulates on the
                    // existing local rotation.
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
fn build_world_ray(resources: &Resources, delta: ViewportInputDelta) -> Option<Ray> {
    let cursor = delta.cursor_local?;
    let viewport_size = delta.viewport_size;
    if viewport_size.x < 1.0 || viewport_size.y < 1.0 {
        return None;
    }
    let aspect = viewport_size.x / viewport_size.y;

    let (camera, gt) = active_camera(resources)?;

    let view = gt.matrix.inverse();
    let proj = ome_render::perspective_rh_reverse_z(
        camera.fov.to_radians(),
        aspect.max(0.001),
        camera.near.max(0.001),
        camera.far.max(camera.near + 0.001),
    );
    let inv_vp = (proj * view).inverse();

    // Cursor in NDC. egui's Y is down; NDC's Y is up.
    let ndc_x = 2.0 * (cursor.x / viewport_size.x) - 1.0;
    let ndc_y = 1.0 - 2.0 * (cursor.y / viewport_size.y);

    // Project a point on the far plane back to world space.
    // Reversed-Z (#488): far plane is ndc.z = 0, near is 1.
    let far_world = inv_vp * Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
    if far_world.w.abs() < 1e-6 {
        return None;
    }
    let far_world = far_world.truncate() / far_world.w;
    let camera_pos = gt.matrix.w_axis.truncate();
    let direction = (far_world - camera_pos).normalize_or_zero();
    if direction == Vec3::ZERO {
        return None;
    }
    Some(Ray::new(camera_pos, direction))
}

fn active_camera(resources: &Resources) -> Option<(PerspectiveCamera, GlobalTransform)> {
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

