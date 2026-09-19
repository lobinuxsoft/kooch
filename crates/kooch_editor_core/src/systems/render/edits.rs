//! Viewport input applied before the same frame's render pass, so a new pose or edit shows at once.

use super::*;

/// Whether the delta moves the editor camera, which keeps the frame pacing continuous.
pub(super) fn drives_camera(input: Option<ViewportInputDelta>) -> bool {
    input.is_some_and(|delta| {
        delta.fly_active
            || delta.fly_keys.any()
            || delta.orbit_yaw != 0.0
            || delta.orbit_pitch != 0.0
            || delta.pan_dx != 0.0
            || delta.pan_dy != 0.0
            || delta.zoom_lines != 0.0
    })
}

pub(super) fn apply_viewport_edits(
    resources: &mut Resources,
    overlay: &mut EditorOverlay,
    input: Option<ViewportInputDelta>,
    actions: &mut Vec<EditorAction>,
) {
    if let Some(mode) = input.and_then(|delta| delta.element_request) {
        overlay.element_mode = mode;
    }
    let playing = resources
        .get::<crate::remote_session::RemoteState>()
        .is_some_and(|state| state.playing);
    // Before anything can drag: a painted element selection under a gated handle looks grabbable
    // and records nothing.
    crate::block_edit::drop_selection_unless_editing(resources, overlay.element_mode, playing);
    // A shape parameter edited in the Inspector reshapes its block this frame.
    if !playing {
        crate::block_edit::shape_sync::sync_block_shapes(resources);
    }
    let Some(delta) = input else {
        return;
    };
    // E with faces selected extrudes; before the handle, which reads the same key as rotate.
    if delta.extrude_pressed
        && overlay.element_mode == crate::block_edit::ElementMode::Face
        && let [entity] = overlay.selected_entities.as_slice()
    {
        extrude(resources, *entity, overlay.snap_settings.translate, actions);
    }

    let selected = overlay.selected_entities.to_vec();
    let snap = overlay.snap_settings;
    // Shape handles first: they sit inside the move arrows' reach, and the more specific one wins.
    let handle_active = crate::gizmos::shape_handles::apply_shape_handles(
        delta, resources, &selected, snap, actions,
    ) || crate::gizmos::apply_handle_input(
        delta,
        resources,
        &selected,
        overlay.rotation_display_mode,
        snap,
        &mut overlay.gizmo_drag_start,
        &mut overlay.shape_drag_start,
        actions,
        overlay.element_mode,
    );
    if handle_active {
        return;
    }
    // Picking only when no handle took the click: a handle sits over what it moves.
    apply_viewport_click(delta, resources, overlay);
    let focus_target = overlay
        .selected_entities
        .first()
        .copied()
        .and_then(|entity| focus_target(resources, entity));
    apply_viewport_input(delta, resources, focus_target);
}

fn extrude(
    resources: &mut Resources,
    entity: kooch_ecs::entity::Entity,
    step: f32,
    actions: &mut Vec<EditorAction>,
) {
    if let Some((before, after)) = crate::block_edit::extrude_selection(resources, entity, step)
        && let Some(source) = crate::block_edit::source_of(resources, entity)
    {
        actions.push(EditorAction::BlockEdit {
            entity,
            source,
            before: Box::new(before),
            after: Box::new(after),
        });
        if let Some(bake) = crate::block_edit::shape_sync::bake(resources, entity) {
            actions.push(bake);
        }
    }
}
