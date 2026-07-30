//! Egui-side input capture: runs **inside** the egui closure and
//! snapshots viewport drag deltas, scroll, modifiers, fly-mode keys,
//! the focus keystroke, and the gizmo-mode hotkeys into a
//! [`ViewportInputDelta`].

use glam::Vec2;

use crate::editor_camera::controller::EditorCameraController;
use crate::editor_camera::fly::FlyKeys;

use super::{HandleModeRequest, ViewportInputDelta};

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
    focused: bool,
) -> ViewportInputDelta {
    let mut delta = ViewportInputDelta::default();
    delta.lmb_clicked = response.clicked();

    // Keys need focus, the pointer needs only hover.
    //
    // Hover alone was the gate, and it is right for the wheel and for a
    // drag — pointing at a panel is how you address it. It is wrong for
    // keys: a pointer resting over the viewport while the user types a name
    // in the Inspector made every `d` both a letter and a step sideways
    // (#661).
    let keys_here = focused && response.hovered();

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

    // --- F key → focus on selection --------------------------------------
    if keys_here {
        delta.focus_pressed = ui.input(|i| i.key_pressed(egui::Key::F));
    }

    // --- W / E / R → handle mode switch ----------------------------------
    //
    // Suppressed during fly mode so the WASD camera movement keys don't
    // accidentally toggle the gizmo mode each time they're tapped.
    if keys_here && !modifiers.any() && !delta.fly_active {
        delta.mode_request = ui.input(|i| {
            if i.key_pressed(egui::Key::W) {
                Some(HandleModeRequest::Translate)
            } else if i.key_pressed(egui::Key::E) {
                Some(HandleModeRequest::Rotate)
            } else if i.key_pressed(egui::Key::R) {
                Some(HandleModeRequest::Scale)
            } else {
                None
            }
        });
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

    // Keyboard modifiers — captured globally (no hover gate) because
    // the user may start a drag, then press Ctrl after the cursor
    // wandered out of the strict hover bounds. egui still reports the
    // modifier state correctly throughout.
    delta.ctrl_held = modifiers.ctrl || modifiers.command;
    delta.shift_held = modifiers.shift;
    delta.alt_held = modifiers.alt;

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
