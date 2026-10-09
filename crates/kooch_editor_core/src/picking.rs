//! Selecting an entity by clicking it in the viewport.

use std::collections::HashMap;

use glam::{Mat3, Mat4, Vec2, Vec3};
use kooch_core::Guid;
use kooch_core::aabb::Aabb;
use kooch_core::resource::Resources;
use kooch_ecs::GlobalTransform;
use kooch_ecs::entity::Entity;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::Query;
use kooch_render::meshlet::MeshletMesh;

/// The entity under `cursor`, or `None` if the click hit nothing.
pub(crate) fn entity_at(
    resources: &mut Resources,
    cursor: Vec2,
    viewport_size: Vec2,
) -> Option<Entity> {
    entity_hit_at(resources, cursor, viewport_size).map(|(entity, _)| entity)
}

/// The entity under `cursor` and how far along the cursor ray it was struck, in world units.
pub(crate) fn entity_hit_at(
    resources: &mut Resources,
    cursor: Vec2,
    viewport_size: Vec2,
) -> Option<(Entity, f32)> {
    surface_at(resources, cursor, viewport_size, &[]).map(|hit| (hit.entity, hit.distance))
}

/// Where the cursor ray meets rendered geometry: what it struck, where, and which way that surface
/// faces.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SurfaceHit {
    pub(crate) entity: Entity,
    /// Distance along the cursor ray, in world units.
    pub(crate) distance: f32,
    pub(crate) point: Vec3,
    /// Unit world-space normal of the surface struck.
    pub(crate) normal: Vec3,
}

/// The nearest rendered surface under `cursor`, skipping `exclude`.
///
/// 🔴 **Rendered meshes, not colliders.** A blockout entity need not have a collider yet, and a
/// snap that only worked on things with physics would be a trap in the exact scene it is for
/// (#1435). The cost is the approximation picking already lives with: a block answers from its
/// triangles, every other mesh from its box.
///
/// 🔴 `exclude` is not optional in practice — a drag that raycasts the thing it is moving snaps it
/// to itself and sticks there. It is a list because an imported model's mesh usually hangs off a
/// CHILD of the entity being dragged, and excluding the parent alone leaves the drag snapping to
/// its own geometry.
pub(crate) fn surface_at(
    resources: &mut Resources,
    cursor: Vec2,
    viewport_size: Vec2,
    exclude: &[Entity],
) -> Option<SurfaceHit> {
    let (camera, transform) = crate::gizmos::active_camera(resources)?;
    let ray = kooch_render::projection::viewport_cursor_to_ray(
        cursor,
        viewport_size,
        transform.matrix,
        camera.fov.to_radians(),
        camera.near,
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
    let mut nearest: Option<SurfaceHit> = None;

    for (entity, mesh, to_world) in candidates {
        if exclude.contains(&entity) {
            continue;
        }
        let aabb = *bounds
            .entry(mesh)
            .or_insert_with(|| local_bounds(resources, mesh));
        let Some(aabb) = aabb else {
            continue;
        };
        let Some(distance) = hit_distance(aabb, to_world, ray.origin, ray.direction) else {
            continue;
        };
        // 🔴 A block's box is not its shape (#1118): a hollow or L-shaped block's box swallows
        // whatever stands inside it, so its triangles decide.
        let (distance, local_normal) =
            match block_hit(resources, mesh, to_world, ray.origin, ray.direction) {
                Some(Some(hit)) => hit,
                Some(None) => continue,
                None => (
                    distance,
                    box_normal(aabb, to_world, ray.origin + ray.direction * distance),
                ),
            };
        if nearest.is_some_and(|best| best.distance <= distance) {
            continue;
        }
        nearest = Some(SurfaceHit {
            entity,
            distance,
            point: ray.origin + ray.direction * distance,
            // The inverse transpose, because a normal is a covector: under a non-uniform scale the
            // matrix that moves the surface tilts its normal the wrong way.
            normal: (normal_matrix(to_world) * local_normal).normalize_or(Vec3::Y),
        });
    }
    nearest
}

/// World matrix for normals: the inverse transpose of the upper 3x3, falling back to the basis
/// itself where it cannot be inverted.
fn normal_matrix(to_world: Mat4) -> Mat3 {
    let basis = Mat3::from_mat4(to_world);
    match basis.determinant().abs() > 1e-12 {
        true => basis.inverse().transpose(),
        false => basis,
    }
}

/// Which face of `aabb` a world-space hit landed on, in the box's own space.
///
/// The dominant axis of the hit measured from the centre in half-extents: on a face that axis
/// reads ±1 while the other two are still inside the box.
fn box_normal(aabb: Aabb, to_world: Mat4, world: Vec3) -> Vec3 {
    let to_local = to_world.inverse();
    if !to_local.is_finite() {
        return Vec3::Y;
    }
    let offset = to_local.transform_point3(world) - aabb.center();
    let half = (aabb.max - aabb.min) * 0.5;
    let share = Vec3::new(
        axis_share(offset.x, half.x),
        axis_share(offset.y, half.y),
        axis_share(offset.z, half.z),
    );
    let (x, y, z) = (share.x.abs(), share.y.abs(), share.z.abs());
    match (x >= y && x >= z, y >= z) {
        (true, _) => Vec3::X * sign(share.x),
        (false, true) => Vec3::Y * sign(share.y),
        (false, false) => Vec3::Z * sign(share.z),
    }
}

/// How far out of the box one axis is, in half-extents. Zero where the extent is too flat to say,
/// which would otherwise divide to infinity and win every comparison.
fn axis_share(offset: f32, half: f32) -> f32 {
    match half > 1e-6 {
        true => offset / half,
        false => 0.0,
    }
}

/// `signum` without its zero case: `0.0_f32.signum()` is `1.0`, but a hit dead on the centre plane
/// has no side, and reading it as positive points the normal into the box half the time.
fn sign(value: f32) -> f32 {
    match value < 0.0 {
        true => -1.0,
        false => 1.0,
    }
}

/// The ray's distance to a block's triangles and the local normal of the face it struck: `None`
/// when `mesh` is not a block, `Some(None)` when the ray misses it.
fn block_hit(
    resources: &Resources,
    mesh: Guid,
    to_world: Mat4,
    origin: Vec3,
    direction: Vec3,
) -> Option<Option<(f32, Vec3)>> {
    let handle = resources
        .get::<kooch_blockmesh::BuiltBlocks>()?
        .handle(mesh)?;
    let assets = resources.get::<kooch_core::assets::Assets<kooch_blockmesh::BlockMesh>>()?;
    let block = assets.get(handle)?;
    let to_local = to_world.inverse();
    if !to_local.is_finite() {
        return Some(None);
    }
    // Unnormalised local direction, so `t` stays comparable with the world-space box distances.
    let origin = to_local.transform_point3(origin);
    let direction = to_local.transform_vector3(direction);
    Some(
        kooch_blockmesh::face_at(block, origin, direction).map(|hit| {
            (
                hit.distance,
                block.face_normal(hit.element as usize).unwrap_or(Vec3::Y),
            )
        }),
    )
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
fn local_bounds(resources: &mut Resources, mesh: Guid) -> Option<Aabb> {
    // 🔴 A generated mesh first, because it has no file to load. A block's renderer names the GUID
    // of the `.block` it was generated from, and asking the server for that produced nothing — so a
    // block was never a candidate and could not be clicked at all.
    if let Some(bounds) = block_bounds(resources, mesh) {
        return Some(bounds);
    }

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

/// The world-space box `entity`'s visual mesh occupies.
pub(crate) fn entity_bounds(resources: &mut Resources, entity: Entity) -> Option<(Vec3, Vec3)> {
    let (mesh, to_world) = visible_meshes(resources)
        .into_iter()
        .find(|(candidate, _, _)| *candidate == entity)
        .map(|(_, mesh, to_world)| (mesh, to_world))?;
    let aabb = local_bounds(resources, mesh)?;

    // Every corner through the transform, not the two extremes: a
    // rotated box's min and max are not its rotated min and max, and
    // framing from those two alone clips the corners that stick out.
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for index in 0..8u32 {
        let corner = Vec3::new(
            if index & 1 == 0 {
                aabb.min.x
            } else {
                aabb.max.x
            },
            if index & 2 == 0 {
                aabb.min.y
            } else {
                aabb.max.y
            },
            if index & 4 == 0 {
                aabb.min.z
            } else {
                aabb.max.z
            },
        );
        let world = to_world.transform_point3(corner);
        min = min.min(world);
        max = max.max(world);
    }
    Some((min, max))
}

/// The bounds of a block's authoring mesh, if this GUID names one.
fn block_bounds(resources: &Resources, mesh: Guid) -> Option<Aabb> {
    let handle = resources
        .get::<kooch_blockmesh::BuiltBlocks>()?
        .handle(mesh)?;
    let assets = resources.get::<kooch_core::assets::Assets<kooch_blockmesh::BlockMesh>>()?;
    let positions = assets.get(handle)?.positions();
    if positions.is_empty() {
        return None;
    }

    let mut min = positions[0];
    let mut max = positions[0];
    for position in positions {
        min = min.min(*position);
        max = max.max(*position);
    }
    Some(Aabb::new(min, max))
}

/// Distance along the world ray at which it enters `aabb`, or `None`.
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
mod tests;
