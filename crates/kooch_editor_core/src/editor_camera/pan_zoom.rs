//! Pan and zoom helpers for the editor camera.

use glam::{Quat, Vec3};

/// World-space delta to apply to `focus_point` for a pan drag.
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
pub fn apply_zoom(distance: f32, scroll_lines: f32, sensitivity: f32) -> f32 {
    distance / sensitivity.powf(scroll_lines)
}

#[cfg(test)]
mod tests;
