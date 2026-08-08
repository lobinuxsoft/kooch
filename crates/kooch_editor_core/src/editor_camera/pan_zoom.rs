//! Pan and zoom helpers for the editor camera.

use glam::{Quat, Vec3};

/// World-space delta to apply to `focus_point` for a pan drag.
///
/// Convention follows Blender / Unity SceneView: dragging right moves
/// the world right under the cursor (camera and pivot translate left).
/// `dx` and `dy` are egui pixel deltas (`+y` is **down** in screen
/// space). `world_units_per_pixel` is the effective sensitivity scaled
/// by current orbit distance — see [`EditorCameraController::effective_pan_sensitivity`].
///
/// [`EditorCameraController::effective_pan_sensitivity`]:
/// crate::editor_camera::EditorCameraController::effective_pan_sensitivity
pub fn pan_delta(
    dx_pixels: f32,
    dy_pixels: f32,
    world_units_per_pixel: f32,
    orientation: Quat,
) -> Vec3 {
    let right = (orientation * Vec3::X).normalize();
    let up = (orientation * Vec3::Y).normalize();
    let dx = dx_pixels * world_units_per_pixel;
    let dy = dy_pixels * world_units_per_pixel;
    // Drag right (+dx) → focus moves left in world.
    // Drag down  (+dy) → focus moves up   in world (camera follows the cursor).
    -right * dx + up * dy
}

/// Returns the new orbit distance after a scroll-zoom event.
///
/// `scroll_lines` is positive when the wheel is rolled away from the
/// user (zoom in). `sensitivity` is a *multiplicative* factor — values
/// just above `1.0` give smooth dolly behaviour (e.g. `1.1` ≈ 10% per
/// notch). The result is **not** clamped; callers should run the value
/// through [`EditorCameraController::clamp_distance`] afterwards.
///
/// [`EditorCameraController::clamp_distance`]:
/// crate::editor_camera::EditorCameraController::clamp_distance
pub fn apply_zoom(distance: f32, scroll_lines: f32, sensitivity: f32) -> f32 {
    distance / sensitivity.powf(scroll_lines)
}

#[cfg(test)]
mod tests;
