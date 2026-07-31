//! Selecting an entity by clicking it in the viewport.
//!
//! # What is picked, and what is not
//!
//! Anything with a **visible** `MeshRenderer` — the same `visible` flag the
//! render pass reads, so what you can click is exactly what you can see. An
//! entity hidden in the Inspector is not silently clickable.
//!
//! Lights, cameras and empties are not picked. They have no geometry; what
//! they draw is a gizmo icon, and clicking those means intersecting the
//! icons rather than the world. That is a different feature and pretending
//! otherwise would make an empty un-clickable *and* look broken.
//!
//! # Bounding boxes, not triangles
//!
//! The test is against each mesh's local-space AABB. Triangle-exact picking
//! means walking meshlets on the CPU or a GPU id-buffer readback — the
//! second is the right answer eventually and the first is the wrong one
//! always. An AABB picks the wrong entity only where two boxes overlap and
//! the nearer one is mostly empty, and it costs one slab test per visible
//! entity.

use std::collections::HashMap;

use glam::{Mat4, Vec2, Vec3};
use kooch_core::Guid;
use kooch_core::aabb::Aabb;
use kooch_core::resource::Resources;
use kooch_ecs::GlobalTransform;
use kooch_ecs::entity::Entity;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::Query;
use kooch_render::meshlet::MeshletMesh;

/// The entity under `cursor`, or `None` if the click hit nothing.
///
/// `cursor` and `viewport_size` are in egui's coordinates — pixels from the
/// viewport's top-left, Y down — the same as
/// [`crate::viewport_pick`] takes.
pub(crate) fn entity_at(
    resources: &mut Resources,
    cursor: Vec2,
    viewport_size: Vec2,
) -> Option<Entity> {
    let (camera, transform) = crate::gizmos::active_camera(resources)?;
    let ray = kooch_render::projection::viewport_cursor_to_ray(
        cursor,
        viewport_size,
        transform.matrix,
        camera.fov.to_radians(),
        camera.near,
        camera.far,
    )?;

    // Collected before resolving any asset: loading a mesh takes
    // `&mut Resources` and the query holds a borrow of it.
    let candidates = visible_meshes(resources);
    if candidates.is_empty() {
        return None;
    }

    // One lookup per distinct mesh rather than per entity — a hundred
    // instances of one tree share a box.
    let mut bounds: HashMap<Guid, Option<Aabb>> = HashMap::new();
    let mut nearest: Option<(f32, Entity)> = None;

    for (entity, mesh, to_world) in candidates {
        let aabb = *bounds
            .entry(mesh)
            .or_insert_with(|| local_bounds(resources, mesh));
        let Some(aabb) = aabb else {
            continue;
        };
        let Some(distance) = hit_distance(aabb, to_world, ray.origin, ray.direction) else {
            continue;
        };
        if nearest.is_none_or(|(best, _)| distance < best) {
            nearest = Some((distance, entity));
        }
    }
    nearest.map(|(_, entity)| entity)
}

/// Every entity the render pass would draw, with its mesh and its world
/// matrix.
fn visible_meshes(resources: &Resources) -> Vec<(Entity, Guid, Mat4)> {
    let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
    let mut out = Vec::new();
    query.for_each_entity(|entity, (renderer, transform)| {
        // The same flag the render pass reads. Picking something invisible
        // would select an entity the user cannot see at the point they
        // clicked.
        if !renderer.visible {
            return;
        }
        if let Some(mesh) = renderer.mesh {
            out.push((entity, mesh, transform.matrix));
        }
    });
    out
}

/// The mesh's local-space bounds.
///
/// `load_by_guid` rather than a read-only lookup: a mesh being drawn is
/// already loaded, so this is a cache hit, and a mesh that is *not* loaded
/// has no bounds to test against anyway.
fn local_bounds(resources: &mut Resources, mesh: Guid) -> Option<Aabb> {
    let mut server = resources.remove::<kooch_core::asset_loader::AssetServer>()?;
    let handle = server.load_by_guid::<MeshletMesh>(mesh, resources).ok();
    resources.insert(server);

    let handle = handle?;
    let assets = resources.get::<kooch_core::assets::Assets<MeshletMesh>>()?;
    let aabb = assets.get(handle)?.aabb;
    // Two `Aabb` types exist — `kooch_render`'s carries mesh bounds and
    // `kooch_core`'s carries the tested slab intersection. Converting is
    // cheaper than a third copy of the same six lines of ray maths.
    Some(Aabb::new(aabb.min, aabb.max))
}

/// Distance along the world ray at which it enters `aabb`, or `None`.
///
/// # Why the ray goes to the box rather than the box to the world
///
/// A rotated entity's world-space AABB is the box *around* its rotated box,
/// which is bigger — sometimes much bigger — and would let a click in empty
/// space next to a diagonal object select it. Transforming the ray into the
/// entity's local space tests the real box, and handles non-uniform scale
/// for free.
///
/// The local direction is deliberately **not** re-normalised: leaving it
/// scaled keeps `t` measured in the world's units, so distances from
/// differently-scaled entities are comparable and the nearest one really is
/// the nearest.
fn hit_distance(aabb: Aabb, to_world: Mat4, origin: Vec3, direction: Vec3) -> Option<f32> {
    let to_local = to_world.inverse();
    if !to_local.is_finite() {
        return None;
    }
    let local_origin = to_local.transform_point3(origin);
    let local_direction = to_local.transform_vector3(direction);
    let (near, far) = aabb.ray_intersect(local_origin, local_direction)?;
    // `near < 0` means the camera is inside the box; the surface the ray
    // actually reaches is the far one. Both behind means the box is behind
    // the camera and was never clicked.
    match (near >= 0.0, far >= 0.0) {
        (true, _) => Some(near),
        (false, true) => Some(far),
        (false, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box() -> Aabb {
        Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5))
    }

    #[test]
    fn a_ray_down_the_axis_hits_a_box_in_front_of_it() {
        let hit = hit_distance(
            unit_box(),
            Mat4::IDENTITY,
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::NEG_Z,
        );
        assert!((hit.unwrap() - 4.5).abs() < 1e-4, "got {hit:?}");
    }

    #[test]
    fn a_ray_that_misses_reports_nothing() {
        assert_eq!(
            hit_distance(
                unit_box(),
                Mat4::IDENTITY,
                Vec3::new(5.0, 5.0, 5.0),
                Vec3::NEG_Z
            ),
            None,
        );
    }

    /// A box behind the camera is not something the user clicked, even
    /// though the infinite line through the cursor passes through it.
    #[test]
    fn a_box_behind_the_camera_is_not_picked() {
        assert_eq!(
            hit_distance(
                unit_box(),
                Mat4::IDENTITY,
                Vec3::new(0.0, 0.0, 5.0),
                Vec3::Z
            ),
            None,
        );
    }

    /// Inside the box, the surface the ray reaches is the far one — and it
    /// is still a hit, not a miss.
    #[test]
    fn standing_inside_a_box_still_picks_it() {
        let hit = hit_distance(unit_box(), Mat4::IDENTITY, Vec3::ZERO, Vec3::NEG_Z);
        assert!((hit.unwrap() - 0.5).abs() < 1e-4, "got {hit:?}");
    }

    /// The whole reason the ray is transformed instead of the box.
    ///
    /// Rotated 45° about Y, the unit box becomes a diamond in the xz
    /// plane — |x| + |z| <= 0.707 — while its world-space AABB is the
    /// square ±0.707. The corner of that square is empty space, so it has
    /// to be aimed at directly: a ray straight down through (0.6, 0.6)
    /// lands in the square but outside the diamond.
    ///
    /// Aiming along -Z instead would prove nothing. Both boxes span the
    /// same ±0.707 in x, so every such ray agrees.
    #[test]
    fn a_rotated_box_is_tested_as_itself_not_as_its_world_bounds() {
        let rotated = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_4);
        assert_eq!(
            hit_distance(unit_box(), rotated, Vec3::new(0.6, 5.0, 0.6), Vec3::NEG_Y),
            None,
            "the inflated world-space box would have swallowed this corner",
        );
        assert!(
            hit_distance(unit_box(), rotated, Vec3::new(0.3, 5.0, 0.3), Vec3::NEG_Y).is_some(),
            "a ray through the real box must still hit",
        );
    }

    /// Scale must not distort the reported distance, or a scaled-up entity
    /// would always claim to be nearer than an unscaled one beside it.
    #[test]
    fn distance_stays_in_world_units_under_scale() {
        let scaled = Mat4::from_scale(Vec3::splat(2.0));
        // Box scaled to ±1 by the transform: the surface is at z = 1, so
        // the ray from z = 5 travels 4 world units.
        let hit = hit_distance(unit_box(), scaled, Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z);
        assert!((hit.unwrap() - 4.0).abs() < 1e-4, "got {hit:?}");
    }

    /// A degenerate transform (zero scale) has no inverse. Testing against
    /// the resulting NaNs would pick unpredictably.
    #[test]
    fn an_entity_with_no_volume_is_not_picked() {
        let collapsed = Mat4::from_scale(Vec3::ZERO);
        assert_eq!(
            hit_distance(unit_box(), collapsed, Vec3::new(0.0, 0.0, 5.0), Vec3::NEG_Z),
            None,
        );
    }
}
