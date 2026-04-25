//! Bridges egui input from the View panel to the editor camera.
//!
//! Two halves:
//!
//! 1. [`collect_viewport_input`] runs **inside** the egui closure with
//!    the View panel's `Response` and `Ui`. It snapshots drag deltas,
//!    scroll, modifier state, fly-mode keys and the focus-on-selection
//!    keystroke into a [`ViewportInputDelta`].
//!
//! 2. [`apply_viewport_input`] runs **outside** the egui closure with
//!    `&mut Resources`. It turns the snapshot into mutations on the
//!    editor camera entity's `Transform` and the [`EditorCameraController`]
//!    resource, then propagates `GlobalTransform` so the renderer sees
//!    the new pose this same frame.

use std::any::TypeId;

use glam::{Quat, Vec2, Vec3};

use ome_core::resource::Resources;
use ome_core::time::Time;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::hierarchy::{GlobalTransform, transform_propagation_system};
use ome_ecs::transform::Transform;

use super::controller::EditorCameraController;
use super::fly::{FlyKeys, fly_velocity};
use super::markers::EditorCamera;
use super::orbit::{apply_yaw_pitch, camera_position, fly_look_pivot_camera};
use super::pan_zoom::{apply_zoom, pan_delta};

/// Snapshot of one frame of viewport-relevant input, captured during
/// the egui pass and consumed afterwards by [`apply_viewport_input`].
///
/// Fields are pre-translated to controller semantics (yaw radians, pan
/// pixels, zoom lines) so the apply step holds no egui types.
#[derive(Debug, Clone, Copy, Default)]
pub struct ViewportInputDelta {
    /// Orbit yaw delta in radians (already multiplied by sensitivity).
    pub orbit_yaw: f32,
    /// Orbit pitch delta in radians (already multiplied by sensitivity).
    pub orbit_pitch: f32,
    /// Pan drag, in egui pixels. `+x` right, `+y` down.
    pub pan_dx: f32,
    pub pan_dy: f32,
    /// Scroll lines (positive = wheel away from user = zoom in).
    pub zoom_lines: f32,
    /// Fly-mode look delta, in radians (already multiplied by sensitivity).
    pub fly_yaw: f32,
    pub fly_pitch: f32,
    /// Fly-mode WASD/QE state. Only meaningful when `fly_active`.
    pub fly_keys: FlyKeys,
    /// `true` when the user is holding RMB inside the viewport.
    pub fly_active: bool,
    /// `true` when the user pressed `F` this frame to focus the selection.
    pub focus_pressed: bool,
    /// Cursor position relative to the viewport's top-left corner, in
    /// physical pixels. `None` when the cursor is outside the viewport.
    /// Consumed by the gizmo handle system to construct picking rays.
    pub cursor_local: Option<Vec2>,
    /// Viewport size in physical pixels, used together with `cursor_local`
    /// to derive normalized device coordinates.
    pub viewport_size: Vec2,
    /// `true` when the primary (left) mouse button was just pressed
    /// this frame inside the viewport.
    pub lmb_pressed: bool,
    /// `true` when the primary (left) mouse button is currently held.
    pub lmb_held: bool,
}

impl ViewportInputDelta {
    /// Returns whether the snapshot would actually change the camera.
    /// Used to skip the entire apply path on idle frames.
    pub fn is_idle(self) -> bool {
        self.orbit_yaw == 0.0
            && self.orbit_pitch == 0.0
            && self.pan_dx == 0.0
            && self.pan_dy == 0.0
            && self.zoom_lines == 0.0
            && self.fly_yaw == 0.0
            && self.fly_pitch == 0.0
            && !self.fly_keys.any()
            && !self.focus_pressed
    }
}

/// Reads egui input within the View panel and returns a delta snapshot.
///
/// Called inside the egui closure with the response of the viewport
/// image (which must have been allocated with `Sense::click_and_drag()`).
///
/// Modifiers and behaviour follow the issue spec:
/// - **MMB drag** → orbit
/// - **Shift + MMB drag** → pan
/// - **Mouse wheel** (only when hovered) → zoom
/// - **RMB hold** → fly mode (mouse delta is look, WASD/QE translate)
/// - **F** (only when hovered) → focus on selection
pub fn collect_viewport_input(
    response: &egui::Response,
    ui: &egui::Ui,
    controller: &EditorCameraController,
) -> ViewportInputDelta {
    let mut delta = ViewportInputDelta::default();

    let modifiers = ui.input(|i| i.modifiers);
    let shift_held = modifiers.shift;

    // --- Middle-mouse drag → orbit or pan ---------------------------------
    if response.dragged_by(egui::PointerButton::Middle) {
        let drag = response.drag_delta();
        if shift_held {
            delta.pan_dx = drag.x;
            delta.pan_dy = drag.y;
        } else {
            // Drag right (+x) yaws the camera right (positive yaw_delta
            // around world +Y is a left turn under right-hand rule, so we
            // negate here to match the "drag-right turns view right" feel).
            delta.orbit_yaw = -drag.x * controller.orbit_sensitivity;
            // Drag down (+y) pitches the camera down (negative pitch).
            delta.orbit_pitch = -drag.y * controller.orbit_sensitivity;
        }
    }

    // --- Right-mouse hold → fly mode --------------------------------------
    //
    // Fly mode stays active for the whole duration RMB is held *after*
    // being pressed inside the viewport. Using `is_pointer_button_down_on`
    // + the global RMB-held query (rather than `dragged_by`) means WASD
    // works on the first frame even before any mouse motion.
    delta.fly_active = response.is_pointer_button_down_on()
        && ui.input(|i| i.pointer.button_down(egui::PointerButton::Secondary));

    if delta.fly_active {
        let drag = response.drag_delta();
        delta.fly_yaw = -drag.x * controller.fly_look_sensitivity;
        delta.fly_pitch = -drag.y * controller.fly_look_sensitivity;
        delta.fly_keys = read_fly_keys(ui);
    }

    // --- Scroll wheel → zoom (only if hovered) ----------------------------
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        // Translate raw pixel scroll into "lines" (egui returns ~30-50 px
        // per mouse-wheel notch on most platforms).
        if scroll.abs() > f32::EPSILON {
            delta.zoom_lines = scroll / 50.0;
        }
    }

    // --- F key → focus on selection (only if hovered) ---------------------
    if response.hovered() {
        delta.focus_pressed = ui.input(|i| i.key_pressed(egui::Key::F));
    }

    // --- Cursor + LMB state for gizmo handles -----------------------------
    let pixels_per_point = ui.ctx().pixels_per_point();
    let rect = response.rect;
    delta.viewport_size = Vec2::new(rect.width(), rect.height()) * pixels_per_point;
    delta.cursor_local = response.hover_pos().map(|p| {
        let local = p - rect.min;
        Vec2::new(local.x, local.y) * pixels_per_point
    });
    let (pressed, held) = ui.input(|i| {
        (
            i.pointer.button_pressed(egui::PointerButton::Primary),
            i.pointer.button_down(egui::PointerButton::Primary),
        )
    });
    // Pressed only counts when the cursor is over the viewport — egui
    // reports the press globally otherwise.
    delta.lmb_pressed = pressed && response.hovered();
    delta.lmb_held = held;

    delta
}

fn read_fly_keys(ui: &egui::Ui) -> FlyKeys {
    ui.input(|i| FlyKeys {
        forward: i.key_down(egui::Key::W),
        backward: i.key_down(egui::Key::S),
        left: i.key_down(egui::Key::A),
        right: i.key_down(egui::Key::D),
        up: i.key_down(egui::Key::E),
        down: i.key_down(egui::Key::Q),
    })
}

/// Applies a captured input delta to the editor camera entity.
///
/// Mutates the controller's focus point / distance, the camera
/// `Transform`, and triggers a hierarchy propagation so the renderer
/// reads the updated `GlobalTransform` on this same frame.
///
/// `selection_world_position` is the world-space position used to
/// re-centre the camera when the user pressed `F`. `None` means "no
/// selection" — focus-on-selection is a no-op in that case.
pub fn apply_viewport_input(
    delta: ViewportInputDelta,
    resources: &mut Resources,
    selection_world_position: Option<Vec3>,
) {
    if delta.is_idle() {
        return;
    }

    let Some(entity) = find_editor_camera_entity(resources) else {
        return;
    };

    let dt = resources
        .get::<Time>()
        .map(|t| t.delta_secs())
        .unwrap_or(1.0 / 60.0);

    // --- Snapshot controller and current transform ------------------------
    let mut controller = match resources.get::<EditorCameraController>() {
        Some(c) => c.clone(),
        None => return,
    };

    // Position is purely derived from focus_point/orientation/distance, so
    // we ignore the existing position and recompute it after mutations.
    let Some((_, mut rotation)) = read_transform(resources, entity) else {
        return;
    };

    // --- Orbit (MMB drag, no Shift) ---------------------------------------
    if delta.orbit_yaw != 0.0 || delta.orbit_pitch != 0.0 {
        rotation = apply_yaw_pitch(rotation, delta.orbit_yaw, delta.orbit_pitch);
    }

    // --- Pan (Shift + MMB drag) -------------------------------------------
    if delta.pan_dx != 0.0 || delta.pan_dy != 0.0 {
        let world_delta = pan_delta(
            delta.pan_dx,
            delta.pan_dy,
            controller.effective_pan_sensitivity(),
            rotation,
        );
        controller.focus_point += world_delta;
    }

    // --- Zoom (mouse wheel) -----------------------------------------------
    if delta.zoom_lines != 0.0 {
        controller.distance =
            apply_zoom(controller.distance, delta.zoom_lines, controller.zoom_sensitivity);
        controller.clamp_distance();
    }

    // --- Fly-mode look + WASD/QE ------------------------------------------
    //
    // FPS look pivots around the *camera*, not around `focus_point`.
    // `fly_look_pivot_camera` rotates and re-anchors `focus_point` so
    // the derived camera position stays fixed under pure rotation.
    // WASD/QE then translates camera and focus together so the in-front
    // pivot moves with the camera.
    if delta.fly_active {
        if delta.fly_yaw != 0.0 || delta.fly_pitch != 0.0 {
            let position_before = camera_position(
                controller.focus_point,
                rotation,
                controller.distance,
            );
            let (new_rotation, new_focus) = fly_look_pivot_camera(
                position_before,
                rotation,
                controller.distance,
                delta.fly_yaw,
                delta.fly_pitch,
            );
            rotation = new_rotation;
            controller.focus_point = new_focus;
        }

        let velocity = fly_velocity(delta.fly_keys, rotation, controller.fly_speed, dt);
        if velocity != Vec3::ZERO {
            controller.focus_point += velocity;
        }
    }

    // --- Focus on selection (F) -------------------------------------------
    if delta.focus_pressed {
        if let Some(target) = selection_world_position {
            controller.focus_point = target;
        }
    }

    // --- Recompute position from the (possibly updated) state -------------
    let position = camera_position(controller.focus_point, rotation, controller.distance);

    write_transform(resources, entity, position, rotation);
    if let Some(c) = resources.get_mut::<EditorCameraController>() {
        *c = controller;
    }

    // Propagate GlobalTransform so the same-frame renderer sees the
    // updated world matrix without waiting for PostUpdate.
    transform_propagation_system(resources);
}

/// Returns the world-space position of `entity` from its `GlobalTransform`,
/// for focus-on-selection. `None` when the entity has no `GlobalTransform`.
pub fn entity_world_position(resources: &Resources, entity: Entity) -> Option<Vec3> {
    let registry = resources.get::<ComponentRegistry>()?;
    let storage = registry.get_cpu::<GlobalTransform>()?;
    let gt = storage.get(entity)?;
    let (_, _, translation) = gt.matrix.to_scale_rotation_translation();
    Some(translation)
}

fn find_editor_camera_entity(resources: &Resources) -> Option<Entity> {
    let archetypes = resources.get::<ArchetypeRegistry>()?;
    let editor_camera_tid = TypeId::of::<EditorCamera>();
    for arch in archetypes.iter_matching(&[]) {
        if arch.components().contains(&editor_camera_tid) {
            return arch.entities().first().copied();
        }
    }
    None
}

fn read_transform(resources: &Resources, entity: Entity) -> Option<(Vec3, Quat)> {
    let registry = resources.get::<ComponentRegistry>()?;
    let storage = registry.get_cpu::<Transform>()?;
    let t = storage.get(entity)?;
    Some((t.position, t.rotation))
}

fn write_transform(resources: &mut Resources, entity: Entity, position: Vec3, rotation: Quat) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Transform>()
        && let Some(t) = storage.get_mut(entity)
    {
        t.position = position;
        t.rotation = rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_delta_does_nothing_on_apply() {
        // No editor camera entity, no resources — must not panic.
        let mut resources = Resources::new();
        apply_viewport_input(ViewportInputDelta::default(), &mut resources, None);
    }

    #[test]
    fn is_idle_detects_default() {
        assert!(ViewportInputDelta::default().is_idle());
    }

    #[test]
    fn is_idle_detects_orbit_input() {
        let mut d = ViewportInputDelta::default();
        d.orbit_yaw = 0.01;
        assert!(!d.is_idle());
    }

    #[test]
    fn is_idle_detects_fly_keys() {
        let mut d = ViewportInputDelta::default();
        d.fly_keys.forward = true;
        assert!(!d.is_idle());
    }

    #[test]
    fn is_idle_detects_focus_press() {
        let mut d = ViewportInputDelta::default();
        d.focus_pressed = true;
        assert!(!d.is_idle());
    }

    #[test]
    fn is_idle_detects_zoom() {
        let mut d = ViewportInputDelta::default();
        d.zoom_lines = -1.0;
        assert!(!d.is_idle());
    }
}
