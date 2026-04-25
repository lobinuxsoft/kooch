//! Editor gizmo system — populates [`GizmoBatch`] from selection state.
//!
//! Architecture: each registered [`Visualizer`] in the
//! [`VisualizerRegistry`] is invoked once per selected entity that has
//! the corresponding component. Built-in visualizers cover Transform,
//! cameras (perspective + orthographic), and directional lights;
//! user-extensibility lands with `ome_editor_api` (phase 4 of #278).
//!
//! Visibility rules (unchanged from PR #277):
//!
//! - **Single selection**: a component's gizmo renders only when its
//!   `CollapsingHeader` is expanded in the Inspector.
//! - **Multi-selection** (>1 entity): only the `Transform` visualizer
//!   runs. Other visualizers are suppressed because they would be
//!   visually ambiguous across multiple entities.

use std::any::TypeId;

use glam::{Mat4, Vec2, Vec3, Vec4};
use ome_core::resource::Resources;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::directional_light::DirectionalLight;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::{GlobalTransform, transform_propagation_system};
use ome_ecs::orthographic_camera::OrthographicCamera;
use ome_ecs::perspective_camera::PerspectiveCamera;
use ome_ecs::query::Query;
use ome_ecs::transform::Transform;
use ome_gizmos::{GizmoBatch, Gizmos, Visualizer, VisualizerRegistry};
use ome_gizmos_handles::{HandleSet, Ray};

use crate::editor_camera::input::ViewportInputDelta;
use crate::state::EditorOverlay;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PLACEHOLDER_BBOX_HALF: f32 = 0.5;
const AXIS_LINE_LENGTH: f32 = 1.0;
const SELECTION_COLOR: Vec3 = Vec3::new(1.0, 0.85, 0.2);

const FRUSTUM_COLOR: Vec3 = Vec3::new(0.4, 0.8, 1.0);
const ORTHO_COLOR: Vec3 = Vec3::new(0.6, 0.85, 1.0);

const DIRLIGHT_ARROW_LENGTH: f32 = 2.0;

/// Aspect ratio used to draw camera frustums. The viewport's actual
/// aspect is not exposed to visualizers in v1 — a fixed 16:9 keeps the
/// frustum shape readable. Future work: read the live aspect from the
/// editor's `ViewportTarget`.
const FRUSTUM_ASPECT: f32 = 16.0 / 9.0;

// ---------------------------------------------------------------------------
// Built-in visualizers
// ---------------------------------------------------------------------------

/// Built-in visualizer for `Transform`: selection bbox + axis arrows.
#[derive(Default)]
pub(crate) struct TransformVisualizer;

impl Visualizer<Transform> for TransformVisualizer {
    fn draw(&self, _component: &Transform, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let origin = transform.matrix.w_axis.truncate();
        // Axis arrows are owned by the translate handles in single-select.
        // The visualizer just draws the selection bbox — multi-select shows
        // only this on every selected entity.
        gizmos.aabb(
            origin - Vec3::splat(PLACEHOLDER_BBOX_HALF),
            origin + Vec3::splat(PLACEHOLDER_BBOX_HALF),
            SELECTION_COLOR,
        );
    }
}

/// Built-in visualizer for `PerspectiveCamera`: pyramid frustum from
/// camera origin to the far plane plus rectangles at near and far.
#[derive(Default)]
pub(crate) struct PerspectiveCameraVisualizer;

impl Visualizer<PerspectiveCamera> for PerspectiveCameraVisualizer {
    fn draw(
        &self,
        camera: &PerspectiveCamera,
        transform: &GlobalTransform,
        gizmos: &mut Gizmos<'_>,
    ) {
        let half_fov = (camera.fov.to_radians() * 0.5).tan();
        let near_h = camera.near * half_fov;
        let near_w = near_h * FRUSTUM_ASPECT;
        let far_h = camera.far * half_fov;
        let far_w = far_h * FRUSTUM_ASPECT;

        // Local-space corners. Camera looks down -Z (right-handed).
        let near = [
            Vec3::new(near_w, near_h, -camera.near),
            Vec3::new(-near_w, near_h, -camera.near),
            Vec3::new(-near_w, -near_h, -camera.near),
            Vec3::new(near_w, -near_h, -camera.near),
        ];
        let far = [
            Vec3::new(far_w, far_h, -camera.far),
            Vec3::new(-far_w, far_h, -camera.far),
            Vec3::new(-far_w, -far_h, -camera.far),
            Vec3::new(far_w, -far_h, -camera.far),
        ];

        let to_world = |p: Vec3| transform.matrix.transform_point3(p);
        let near_w: [Vec3; 4] = [to_world(near[0]), to_world(near[1]), to_world(near[2]), to_world(near[3])];
        let far_w: [Vec3; 4] = [to_world(far[0]), to_world(far[1]), to_world(far[2]), to_world(far[3])];

        // Near rectangle.
        for i in 0..4 {
            gizmos.line(near_w[i], near_w[(i + 1) % 4], FRUSTUM_COLOR);
        }
        // Far rectangle.
        for i in 0..4 {
            gizmos.line(far_w[i], far_w[(i + 1) % 4], FRUSTUM_COLOR);
        }
        // Connecting edges from near to far (the 4 frustum side edges).
        for i in 0..4 {
            gizmos.line(near_w[i], far_w[i], FRUSTUM_COLOR);
        }
    }
}

/// Built-in visualizer for `OrthographicCamera`: 12-edge wireframe box
/// of the orthographic volume.
#[derive(Default)]
pub(crate) struct OrthographicCameraVisualizer;

impl Visualizer<OrthographicCamera> for OrthographicCameraVisualizer {
    fn draw(
        &self,
        camera: &OrthographicCamera,
        transform: &GlobalTransform,
        gizmos: &mut Gizmos<'_>,
    ) {
        let half_w = camera.size * FRUSTUM_ASPECT;
        let half_h = camera.size;

        // 8 corners in local space (camera looks -Z).
        let corners_local = [
            Vec3::new(half_w, half_h, -camera.near),
            Vec3::new(-half_w, half_h, -camera.near),
            Vec3::new(-half_w, -half_h, -camera.near),
            Vec3::new(half_w, -half_h, -camera.near),
            Vec3::new(half_w, half_h, -camera.far),
            Vec3::new(-half_w, half_h, -camera.far),
            Vec3::new(-half_w, -half_h, -camera.far),
            Vec3::new(half_w, -half_h, -camera.far),
        ];

        let c: [Vec3; 8] = std::array::from_fn(|i| transform.matrix.transform_point3(corners_local[i]));

        // Near rect, far rect, and 4 side edges.
        for i in 0..4 {
            gizmos.line(c[i], c[(i + 1) % 4], ORTHO_COLOR);
            gizmos.line(c[4 + i], c[4 + (i + 1) % 4], ORTHO_COLOR);
            gizmos.line(c[i], c[4 + i], ORTHO_COLOR);
        }
    }
}

/// Built-in visualizer for `DirectionalLight`: a single arrow pointing
/// along the light's forward direction (entity's -Z axis), tinted by
/// the light's color.
#[derive(Default)]
pub(crate) struct DirectionalLightVisualizer;

impl Visualizer<DirectionalLight> for DirectionalLightVisualizer {
    fn draw(
        &self,
        light: &DirectionalLight,
        transform: &GlobalTransform,
        gizmos: &mut Gizmos<'_>,
    ) {
        let origin = transform.matrix.w_axis.truncate();
        let forward = transform
            .matrix
            .transform_vector3(Vec3::NEG_Z)
            .normalize_or_zero();
        if forward == Vec3::ZERO {
            return;
        }
        // Two perpendicular axes for the arrowhead — derive from forward
        // and a stable up reference.
        let up_ref = if forward.y.abs() > 0.99 {
            Vec3::X
        } else {
            Vec3::Y
        };
        let right = forward.cross(up_ref).normalize_or(Vec3::X);
        let up = right.cross(forward).normalize_or(Vec3::Y);

        let tip = origin + forward * DIRLIGHT_ARROW_LENGTH;
        gizmos.arrow(origin, tip, right, up, light.color);
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Inserts the `VisualizerRegistry` and registers built-in visualizers.
/// Also inserts a default `HandleSet` (3 translate handles X/Y/Z).
/// Runs once at editor startup.
pub(crate) fn register_builtin_visualizers_system(resources: &mut Resources) {
    let mut registry = resources
        .remove::<VisualizerRegistry>()
        .unwrap_or_default();
    registry.register::<Transform, TransformVisualizer>();
    registry.register::<PerspectiveCamera, PerspectiveCameraVisualizer>();
    registry.register::<OrthographicCamera, OrthographicCameraVisualizer>();
    registry.register::<DirectionalLight, DirectionalLightVisualizer>();
    resources.insert(registry);

    if resources.get::<HandleSet>().is_none() {
        resources.insert(HandleSet::default());
    }
}

/// Pre-render system that rebuilds the gizmo batch from current
/// selection by dispatching through the [`VisualizerRegistry`].
pub(crate) fn build_gizmo_batch_system(resources: &mut Resources) {
    let (selected, ctx) = match resources.get::<EditorOverlay>() {
        Some(overlay) => (overlay.selected_entities.clone(), overlay.ctx.clone()),
        None => return,
    };

    let mut batch = resources.remove::<GizmoBatch>().unwrap_or_default();
    batch.clear();

    if selected.is_empty() {
        resources.insert(batch);
        return;
    }

    let multi = selected.len() > 1;
    let transform_type_id = TypeId::of::<Transform>();

    let registry = resources
        .remove::<VisualizerRegistry>()
        .unwrap_or_default();

    {
        let mut gizmos = Gizmos::new(&mut batch);
        let resources_ref: &Resources = &*resources;
        for entity in &selected {
            for type_id in registry.registered_types() {
                if multi && type_id != transform_type_id {
                    continue;
                }
                if !multi && !is_component_expanded(&ctx, *entity, type_id) {
                    continue;
                }
                registry.dispatch(type_id, *entity, resources_ref, &mut gizmos);
            }
        }
    }

    // Pass 5: handles. They draw on top of the visualizers, with hover/
    // drag state managed by `HandleSet`. The handle set is populated
    // (origin updated) only when exactly one entity is selected — multi
    // and empty selections suppress translate handles for v1.
    if let Some(handle_set) = resources.get::<HandleSet>() {
        let mut gizmos = Gizmos::new(&mut batch);
        handle_set.draw(&mut gizmos);
    }

    resources.insert(batch);
    resources.insert(registry);
}

/// Updates `HandleSet` from this frame's viewport input and applies any
/// resulting translation delta to the (single) selected entity. Runs
/// inside `editor_render_system` between input capture and camera
/// input apply, so the handle can absorb input before the camera
/// controller sees it.
///
/// Returns `true` when the handle is hovered or dragging — the caller
/// should skip applying camera-controller input on those frames.
pub(crate) fn apply_handle_input(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    selected: &[Entity],
) -> bool {
    // Single-entity v1: handles are suppressed for empty / multi selection.
    let target = match selected {
        [e] => *e,
        _ => {
            // Reset state so leftover hover/drag clears when selection changes.
            if let Some(handle_set) = resources.get_mut::<HandleSet>() {
                let _ = handle_set.update(None, false, false);
            }
            return false;
        }
    };

    let target_origin = match entity_world_position(resources, target) {
        Some(p) => p,
        None => return false,
    };

    let ray = build_world_ray(resources, delta);

    let mut handle_set = match resources.remove::<HandleSet>() {
        Some(h) => h,
        None => return false,
    };
    handle_set.set_origin(target_origin);
    let translation = handle_set.update(ray, delta.lmb_pressed, delta.lmb_held);
    let active = handle_set.is_active();
    let dragging = handle_set.is_dragging();
    resources.insert(handle_set);

    // Apply the per-frame delta to the entity's local Transform.
    // We mutate `Transform.position`; `transform_propagation_system`
    // re-derives the world matrix downstream.
    if dragging && translation != Vec3::ZERO {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<Transform>()
            && let Some(t) = storage.get_mut(target)
        {
            t.position += translation;
        }
        // Re-propagate so the same-frame render sees the new world matrix.
        transform_propagation_system(resources);
    }

    active
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
    let proj = Mat4::perspective_rh(
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
    let far_world = inv_vp * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
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

fn entity_world_position(resources: &Resources, entity: Entity) -> Option<Vec3> {
    let registry = resources.get::<ComponentRegistry>()?;
    let storage = registry.get_cpu::<GlobalTransform>()?;
    let gt = storage.get(entity)?;
    Some(gt.matrix.w_axis.truncate())
}

/// Reads the Inspector's `CollapsingHeader` state for a (entity,
/// component) pair. Defaults to `true` (open) when no state is stored.
fn is_component_expanded(ctx: &egui::Context, entity: Entity, type_id: TypeId) -> bool {
    let id = egui::Id::new(format!("comp_{}_{:?}", entity.index(), type_id));
    egui::collapsing_header::CollapsingState::load(ctx, id)
        .map(|state| state.is_open())
        .unwrap_or(true)
}
