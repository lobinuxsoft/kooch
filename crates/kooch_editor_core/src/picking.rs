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

    // 🔴 Two passes, because resolving a mesh needs `&mut Resources` and reading its triangles
    // needs a borrow of the asset store that lives across the whole test. One pass cannot hold
    // both, and loading a mesh lazily inside the test loop is what the borrow checker was
    // objecting to.
    //
    // One entry per distinct mesh rather than per entity — a hundred instances of one tree share a
    // shape.
    let mut shapes: HashMap<Guid, Option<Shape>> = HashMap::new();
    for (_, mesh, _) in &candidates {
        if !shapes.contains_key(mesh) {
            shapes.insert(*mesh, resolve_shape(resources, *mesh));
        }
    }

    let mut nearest: Option<SurfaceHit> = None;
    for (entity, mesh, to_world) in candidates {
        if exclude.contains(&entity) {
            continue;
        }
        let Some(shape) = shapes.get(&mesh).copied().flatten() else {
            continue;
        };
        // The box first, always: it rejects most of the scene for the price of six comparisons,
        // and the triangle test below only runs on what survives.
        let Some(entry) = hit_distance(shape.aabb, to_world, ray.origin, ray.direction) else {
            continue;
        };
        if nearest.is_some_and(|best| best.distance < entry) {
            continue;
        }
        let Some((distance, local_normal)) =
            exact_hit(resources, shape, to_world, ray.origin, ray.direction)
        else {
            continue;
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

/// A mesh resolved to something a ray can be tested against: its box, and where its triangles are.
#[derive(Debug, Clone, Copy)]
struct Shape {
    aabb: Aabb,
    triangles: Triangles,
}

/// Which store a mesh's triangles come from.
#[derive(Debug, Clone, Copy)]
enum Triangles {
    /// A block authored in the editor. Its faces are the shape (#1118).
    Block(kooch_core::assets::Handle<kooch_blockmesh::BlockMesh>),
    /// An imported mesh. LOD 0 of its meshlet chain is the shape.
    Meshlet(kooch_core::assets::Handle<MeshletMesh>),
}

/// Where the ray truly meets `shape`, with the local normal of what it struck.
///
/// 🔴 Triangles, never the box. A sphere's box answers a ray a long way from the sphere, and a snap
/// that trusts it puts things in mid air (#1435). Picking tolerated it because selecting roughly
/// the right object is still the right object; placing on it does not.
fn exact_hit(
    resources: &Resources,
    shape: Shape,
    to_world: Mat4,
    origin: Vec3,
    direction: Vec3,
) -> Option<(f32, Vec3)> {
    let to_local = to_world.inverse();
    if !to_local.is_finite() {
        return None;
    }
    // Unnormalised local direction, so `t` stays comparable with the world-space box distances
    // every other candidate is measured in.
    let origin = to_local.transform_point3(origin);
    let direction = to_local.transform_vector3(direction);

    match shape.triangles {
        Triangles::Block(handle) => {
            let assets =
                resources.get::<kooch_core::assets::Assets<kooch_blockmesh::BlockMesh>>()?;
            let block = assets.get(handle)?;
            let hit = kooch_blockmesh::face_at(block, origin, direction)?;
            Some((
                hit.distance,
                block.face_normal(hit.element as usize).unwrap_or(Vec3::Y),
            ))
        }
        Triangles::Meshlet(handle) => {
            let assets = resources.get::<kooch_core::assets::Assets<MeshletMesh>>()?;
            let mesh = assets.get(handle)?;
            let hit = kooch_render::meshlet::ray_hit(mesh, origin, direction)?;
            Some((hit.distance, hit.normal))
        }
    }
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

/// A mesh resolved to its box and its triangles, loading it if this is the first ask.
fn resolve_shape(resources: &mut Resources, mesh: Guid) -> Option<Shape> {
    // 🔴 A generated mesh first, because it has no file to load. A block's renderer names the GUID
    // of the `.block` it was generated from, and asking the server for that produced nothing — so a
    // block was never a candidate and could not be clicked at all.
    if let Some(handle) = resources
        .get::<kooch_blockmesh::BuiltBlocks>()
        .and_then(|built| built.handle(mesh))
    {
        return Some(Shape {
            aabb: block_bounds(resources, mesh)?,
            triangles: Triangles::Block(handle),
        });
    }

    let mut server = resources.remove::<kooch_core::asset_loader::AssetServer>()?;
    let handle = server.load_by_guid::<MeshletMesh>(mesh, resources).ok();
    resources.insert(server);

    let handle = handle?;
    let assets = resources.get::<kooch_core::assets::Assets<MeshletMesh>>()?;
    let aabb = assets.get(handle)?.aabb;
    Some(Shape {
        // Two `Aabb` types exist — `kooch_render`'s carries mesh bounds and `kooch_core`'s carries
        // the tested slab intersection. Converting is cheaper than a third copy of the same six
        // lines of ray maths.
        aabb: Aabb::new(aabb.min, aabb.max),
        triangles: Triangles::Meshlet(handle),
    })
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
