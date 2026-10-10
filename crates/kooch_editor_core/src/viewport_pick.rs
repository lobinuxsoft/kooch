//! Turning a place on screen into a place in the world.

use glam::{Vec2, Vec3};
use kooch_core::resource::Resources;

/// Where something dropped into the editor should end up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DropPoint {
    /// Leave it wherever it was authored.
    Authored,
    /// Under the cursor in a viewport, in egui's coordinates — pixels from
    /// the viewport's top-left, Y down.
    Viewport { cursor: Vec2, viewport_size: Vec2 },
}

/// How far in front of the camera to place something when the ground is not in view.
const FALLBACK_DISTANCE: f32 = 10.0;

/// Resolves a [`DropPoint`] to a world position.
///
/// 🔴 The geometry first, then the ground, then a guess. A drop that only knew the plane at
/// `y = 0` put a prefab released over a hillside at sea level, through the hill (#1459).
///
/// `&mut` because resolving a mesh may load it — the same reason [`crate::picking`] takes one.
pub(crate) fn resolve(resources: &mut Resources, point: DropPoint) -> Option<Vec3> {
    let DropPoint::Viewport {
        cursor,
        viewport_size,
    } = point
    else {
        return None;
    };

    if let Some(hit) = crate::picking::surface_at(resources, cursor, viewport_size, &[]) {
        return Some(hit.point);
    }

    let (camera, transform) = crate::gizmos::active_camera(resources)?;
    let ray = kooch_render::projection::viewport_cursor_to_ray(
        cursor,
        viewport_size,
        transform.matrix,
        camera.fov.to_radians(),
        camera.near,
    )?;

    if let Some(hit) = ray.hits_horizontal_plane(0.0) {
        return Some(hit);
    }
    let distance = resources
        .get::<crate::editor_camera::EditorCameraController>()
        .map(|controller| controller.distance)
        .unwrap_or(FALLBACK_DISTANCE);
    Some(ray.at(distance))
}

#[cfg(test)]
mod tests;
