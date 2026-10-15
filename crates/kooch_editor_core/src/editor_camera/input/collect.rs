//! Egui-side input capture: runs **inside** the egui closure and snapshots viewport drag deltas,
//! scroll, modifiers, fly-mode keys, the focus keystroke, and the gizmo-mode hotkeys into a
//! [`ViewportInputDelta`].

use glam::Vec2;

use crate::commands;
use crate::editor_camera::controller::EditorCameraController;
use crate::editor_camera::fly::FlyKeys;

use super::{HandleModeRequest, ViewportInputDelta};

/// Reads egui input within the View panel and returns a delta snapshot.
pub fn collect_viewport_input(
    response: &egui::Response,
    ui: &egui::Ui,
    controller: &EditorCameraController,
    focused: bool,
) -> ViewportInputDelta {
    let mut delta = ViewportInputDelta::default();
    delta.lmb_clicked = response.clicked();

    // Keys need focus, the pointer needs only hover.
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

    // --- Keyboard commands ------------------------------------------------
    // 🔴 Asked of `crate::commands`, not written here: the overlay prints the same values, and a
    // key spelled twice is a key free to disagree with its own label (#1436).
    if keys_here {
        delta.focus_pressed = ui.input(|i| commands::FOCUS.pressed(i));
        delta.extrude_pressed = ui.input(|i| commands::EXTRUDE.pressed(i));
    }

    // 🔴 Gated on `!fly_active` because `A` is also strafe-left: without this, flying sideways
    // with a spline selected drops a knot on every press of the key that moves the camera.
    if keys_here && !delta.fly_active {
        delta.add_knot_pressed = ui.input(|i| commands::ADD_KNOT.pressed(i));
    }

    // --- Handle mode switch ----------------------------------------------
    if keys_here && !delta.fly_active {
        delta.mode_request = ui.input(|i| {
            [
                (commands::MOVE, HandleModeRequest::Translate),
                (commands::ROTATE, HandleModeRequest::Rotate),
                (commands::SCALE, HandleModeRequest::Scale),
            ]
            .into_iter()
            .find(|(command, _)| command.pressed(i))
            .map(|(_, request)| request)
        });
    }

    // --- What a click selects inside a block ------------------------------
    if keys_here && !delta.fly_active {
        delta.element_request = ui.input(|i| {
            crate::block_edit::ElementMode::ALL
                .iter()
                .zip(commands::ELEMENTS)
                .find(|(_, command)| command.pressed(i))
                .map(|(mode, _)| *mode)
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

    // Keyboard modifiers — captured globally (no hover gate) because the user may start a drag,
    // then press Ctrl after the cursor wandered out of the strict hover bounds. egui still reports
    // the modifier state correctly throughout.
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
