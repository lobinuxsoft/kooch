//! Editor gizmo system — populates [`GizmoBatch`] from selection state.
//!
//! Architecture: each registered [`Visualizer`] in the
//! [`VisualizerRegistry`] is invoked once per selected entity that has
//! the corresponding component. The selection bbox + axis arrows
//! historically hardcoded here are now expressed as a built-in
//! [`TransformVisualizer`] registered alongside any user-provided
//! visualizers (the user-extensibility surface lands in phase 4 with
//! `ome_editor_api`).
//!
//! Visibility rules (unchanged from PR #277):
//!
//! - **Single selection**: a component's gizmo renders only when its
//!   `CollapsingHeader` is expanded in the Inspector.
//! - **Multi-selection** (>1 entity): only the `Transform` visualizer
//!   runs. Other visualizers are suppressed because they would be
//!   visually ambiguous across multiple entities.

use std::any::TypeId;

use glam::Vec3;
use ome_core::resource::Resources;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::transform::Transform;
use ome_gizmos::{GizmoBatch, Gizmos, Visualizer, VisualizerRegistry};

use crate::state::EditorOverlay;

/// Built-in visualizer for `Transform`: the standard selection bbox +
/// world-space axis arrows that the editor has shown since #270.
#[derive(Default)]
pub(crate) struct TransformVisualizer;

const PLACEHOLDER_BBOX_HALF: f32 = 0.5;
const AXIS_LINE_LENGTH: f32 = 1.0;
const SELECTION_COLOR: Vec3 = Vec3::new(1.0, 0.85, 0.2);

impl Visualizer<Transform> for TransformVisualizer {
    fn draw(&self, _component: &Transform, transform: &GlobalTransform, gizmos: &mut Gizmos<'_>) {
        let origin = transform.matrix.w_axis.truncate();
        gizmos.aabb(
            origin - Vec3::splat(PLACEHOLDER_BBOX_HALF),
            origin + Vec3::splat(PLACEHOLDER_BBOX_HALF),
            SELECTION_COLOR,
        );
        gizmos.axis_arrows(origin, AXIS_LINE_LENGTH);
    }
}

/// Inserts the `VisualizerRegistry` and registers built-in visualizers.
/// Runs once at editor startup.
pub(crate) fn register_builtin_visualizers_system(resources: &mut Resources) {
    let mut registry = resources
        .remove::<VisualizerRegistry>()
        .unwrap_or_default();
    registry.register::<Transform, TransformVisualizer>();
    resources.insert(registry);
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
                // Multi-select forces Transform-only.
                if multi && type_id != transform_type_id {
                    continue;
                }
                // Single-select gates on Inspector expansion. Transform
                // matters here: a collapsed Transform hides its arrows.
                if !multi && !is_component_expanded(&ctx, *entity, type_id) {
                    continue;
                }
                registry.dispatch(type_id, *entity, resources_ref, &mut gizmos);
            }
        }
    }

    resources.insert(batch);
    resources.insert(registry);
}

/// Reads the Inspector's `CollapsingHeader` state for a (entity,
/// component) pair. Returns the previous frame's open/closed state,
/// defaulting to `true` (open) when no state is stored — matches the
/// Inspector's `CollapsingState::load_with_default_open(_, _, true)`.
fn is_component_expanded(ctx: &egui::Context, entity: Entity, type_id: TypeId) -> bool {
    let id = egui::Id::new(format!("comp_{}_{:?}", entity.index(), type_id));
    egui::collapsing_header::CollapsingState::load(ctx, id)
        .map(|state| state.is_open())
        .unwrap_or(true)
}
