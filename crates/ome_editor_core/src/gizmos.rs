//! Editor gizmo system — populates [`GizmoBatch`] from selection state.
//!
//! Runs at `Stage::PreRender` after transform propagation has finished.
//! Visibility rules:
//!
//! - **Single selection**: a component's gizmo is drawn only when its
//!   `CollapsingHeader` is expanded in the Inspector. This ties gizmo
//!   visibility to user attention — collapse a component to hide its
//!   gizmo, expand to show it.
//! - **Multi-selection** (>1 entity selected): only Transform gizmos
//!   render, regardless of expansion state. Other component visualizers
//!   (camera frustum, light direction, …) are suppressed because they
//!   would be visually ambiguous across multiple entities.
//!
//! AABB sizing is a fixed 1 m³ for now. Component-aware bounds (sphere
//! radius, mesh AABB, SDF box extents) will land in the per-gizmo
//! issues that build on this foundation.
//!
//! For now only Transform has a gizmo. Camera frustum / light arrow live
//! in #274 (visualizers issue) and will register here once landed.

use std::any::TypeId;

use glam::Vec3;
use ome_core::resource::Resources;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;
use ome_ecs::transform::Transform;
use ome_gizmos::GizmoBatch;

use crate::state::EditorOverlay;

const PLACEHOLDER_BBOX_HALF: f32 = 0.5;
const AXIS_LINE_LENGTH: f32 = 1.0;
const SELECTION_COLOR: Vec3 = Vec3::new(1.0, 0.85, 0.2);

/// Pre-render system that rebuilds the gizmo batch from current selection.
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

    let query = Query::<&GlobalTransform>::new(resources);
    for entity in &selected {
        let Some(global) = query.get(*entity) else {
            continue;
        };
        // Multi-select forces Transform-only; single-select gates on the
        // Transform component being expanded in the Inspector.
        let draw_transform = multi
            || is_component_expanded(&ctx, *entity, transform_type_id);
        if draw_transform {
            let origin = global.matrix.w_axis.truncate();
            batch.aabb(
                origin - Vec3::splat(PLACEHOLDER_BBOX_HALF),
                origin + Vec3::splat(PLACEHOLDER_BBOX_HALF),
                SELECTION_COLOR,
            );
            batch.axis_arrows(origin, AXIS_LINE_LENGTH);
        }
    }
    drop(query);

    resources.insert(batch);
}

/// Reads the Inspector's `CollapsingHeader` state for a (entity, component)
/// pair. Returns the previous frame's open/closed state, defaulting to
/// `true` (open) when no state is stored — matches the Inspector's
/// `CollapsingState::load_with_default_open(ctx, id, true)` call.
fn is_component_expanded(ctx: &egui::Context, entity: Entity, type_id: TypeId) -> bool {
    // Must match the ID format in `panels::inspector::mod.rs::draw`.
    let id = egui::Id::new(format!("comp_{}_{:?}", entity.index(), type_id));
    egui::collapsing_header::CollapsingState::load(ctx, id)
        .map(|state| state.is_open())
        .unwrap_or(true)
}
