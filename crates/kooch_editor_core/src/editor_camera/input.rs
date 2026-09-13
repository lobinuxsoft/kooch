//! Bridges egui input from the View panel to the editor camera.

mod apply;
mod collect;

use glam::Vec2;

use crate::editor_camera::fly::FlyKeys;

pub use apply::{apply_viewport_input, entity_world_position};
pub use collect::collect_viewport_input;

/// Mode request raised by W / E / R hotkeys. Mirrors
/// `kooch_gizmos_handles::HandleMode` but kept local to avoid pulling
/// the handles crate into this module's surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleModeRequest {
    Translate,
    Rotate,
    Scale,
}

/// Snapshot of one frame of viewport-relevant input, captured during the egui pass and consumed
/// afterwards by [`apply_viewport_input`].
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
    /// E with faces selected: pull them out along their averaged
    /// normal. The letter ProBuilder and Blender both use.
    pub extrude_pressed: bool,
    /// `Some(mode)` when the user pressed W / E / R this frame inside
    /// the viewport. Forwarded to `HandleSet::set_mode`.
    pub mode_request: Option<HandleModeRequest>,
    /// `Some(mode)` when the user pressed 1 / 2 / 3 / 4 this frame,
    /// switching what a click in a block selects.
    pub(crate) element_request: Option<crate::block_edit::ElementMode>,
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
    /// `true` when the viewport received a *click* — pressed and released without dragging.
    pub lmb_clicked: bool,
    /// Keyboard modifier state at this frame. Threaded through to the
    /// gizmo handle system for snap modifiers (Ctrl on translate,
    /// Shift on rotate).
    pub ctrl_held: bool,
    pub shift_held: bool,
    pub alt_held: bool,
    /// `Some(orientation)` when the axis gizmo was clicked this frame:
    /// the camera snaps to that orientation, keeping its focus point
    /// and distance — Godot's navigation-gizmo behaviour.
    pub snap_orientation: Option<glam::Quat>,
}

impl ViewportInputDelta {
    /// Returns whether the snapshot would actually change the camera. Used to skip the entire apply
    /// path on idle frames.
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
            && !self.extrude_pressed
            && self.snap_orientation.is_none()
    }
}

#[cfg(test)]
mod tests;
