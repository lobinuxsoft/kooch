//! Turning a place on screen into a place in the world.
//!
//! # Why this is not resolved in the panel that reports it
//!
//! Unprojecting needs the camera's orientation, which lives on the camera
//! entity's `Transform` — not on `EditorCameraController`, which knows only
//! where the camera is looking and how far away it is. A panel draws with
//! `&mut Ui` and has no world to read, so it reports *where on screen*
//! something happened and a handler with `Resources` resolves it.
//!
//! That is the same split `ViewportInputDelta` already uses for gizmo
//! dragging, and it means the drop path and the gizmo path agree about
//! which camera they are talking about.

use glam::{Vec2, Vec3};
use ome_core::resource::Resources;

/// Where something dropped into the editor should end up.
///
/// A variant per *kind of answer*, not per panel: two panels that name no
/// place both mean [`Self::Authored`], and any panel that later grows a
/// viewport gets [`Self::Viewport`] for free.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DropPoint {
    /// Leave it wherever it was authored.
    ///
    /// What the World panel and the Asset Browser's context menu mean: a
    /// hierarchy list and a menu item name no location, and inventing one
    /// (the origin, say) would move a prefab that was deliberately
    /// authored somewhere.
    Authored,
    /// Under the cursor in a viewport, in egui's coordinates — pixels from
    /// the viewport's top-left, Y down.
    Viewport { cursor: Vec2, viewport_size: Vec2 },
}

/// How far in front of the camera to place something when the ground is
/// not in view.
///
/// Only used if the editor camera cannot be found at all; normally the
/// camera's own focus distance is used, which puts the object where the
/// user is already looking.
const FALLBACK_DISTANCE: f32 = 10.0;

/// Resolves a [`DropPoint`] to a world position.
///
/// `None` for [`DropPoint::Authored`] — there is nothing to resolve, and
/// the caller's `Option<Vec3>` already means "leave it".
///
/// # Where a viewport drop lands
///
/// The cursor gives a ray, not a point, so something has to choose the
/// distance along it. In order:
///
/// 1. **The ground plane at y = 0.** What the user means when they drop
///    onto visible ground, and it matches where the grid is drawn.
/// 2. **The camera's focus distance.** Reached when the ray misses the
///    plane — looking up at the sky, or level with the horizon. Placing
///    the object where the camera is already focused is wrong less often
///    than refusing to place it, which reads as the drop not working.
///
/// Neither step queries actual geometry: dropping onto a *surface* needs
/// scene queries (#562). Until then the ground plane is the honest
/// approximation, and it is exactly right for the flat-plane scenes this
/// is being used with.
pub(crate) fn resolve(resources: &Resources, point: DropPoint) -> Option<Vec3> {
    let DropPoint::Viewport {
        cursor,
        viewport_size,
    } = point
    else {
        return None;
    };

    let (camera, transform) = crate::gizmos::active_camera(resources)?;
    let ray = ome_render::projection::viewport_cursor_to_ray(
        cursor,
        viewport_size,
        transform.matrix,
        camera.fov.to_radians(),
        camera.near,
        camera.far,
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
mod tests {
    use super::*;

    /// `Authored` has nothing to resolve, and must not accidentally become
    /// a position — that would move every prefab dropped in the World panel.
    #[test]
    fn an_authored_drop_resolves_to_nothing() {
        let resources = Resources::new();
        assert_eq!(resolve(&resources, DropPoint::Authored), None);
    }

    /// A queryable world with nothing in it. `Query::new` requires the
    /// registries — a `Resources` without them is not an empty world, it is
    /// not a world — so this is what "no camera" actually looks like.
    fn empty_world() -> Resources {
        let mut resources = Resources::new();
        resources.insert(ome_ecs::component::ComponentRegistry::new());
        resources.insert(ome_ecs::archetype_registry::ArchetypeRegistry::new());
        resources.insert(ome_ecs::query::AccessTracker::new());
        resources
    }

    /// With no camera there is nothing to unproject against. Returning a
    /// position anyway would place the object at a made-up point.
    #[test]
    fn a_viewport_drop_with_no_camera_resolves_to_nothing() {
        let point = DropPoint::Viewport {
            cursor: Vec2::new(10.0, 10.0),
            viewport_size: Vec2::new(800.0, 600.0),
        };
        assert_eq!(resolve(&empty_world(), point), None);
    }
}
