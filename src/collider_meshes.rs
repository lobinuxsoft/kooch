//! [`ColliderMeshPlugin`] fills [`ColliderMeshCache`](kooch_physics::ColliderMeshCache) here, where
//! assets and physics meet, keeping [`PhysicsBackend`](kooch_physics::PhysicsBackend) off wgpu. It
//! parses the `.glb` (36 ms for 76k vertices), not a `MeshletMesh` whose LOD chain costs 2.9 s.

use std::path::{Path, PathBuf};

use kooch_core::Guid;
use kooch_core::app::App;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::asset_meta;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;
use kooch_physics::components::{Collider, SHAPE_CONVEX_HULL, is_mesh_derived};
use kooch_physics::{ColliderMesh, ColliderMeshCache, ConvexPart, hull_of};
use kooch_render::mesh::{parse_mesh_bytes_full, parse_mesh_parts};

/// The `[import]` key a baked collision asset carries; in the sidecar, since artist meshes have one
/// primitive per material, not per convex piece.
pub const COLLISION_KEY: &str = "collision";
pub const COLLISION_PARTS: &str = "parts";
pub const COLLISION_HULL: &str = "hull";
/// A decimated copy of the triangles. 🔴 Not trusted as convex faces: a simplified mesh is still
/// concave or open.
pub const COLLISION_MESH: &str = "mesh";

/// Resolves the meshes mesh-derived colliders name.
pub struct ColliderMeshPlugin;

impl Plugin for ColliderMeshPlugin {
    fn build(&self, app: &mut App) {
        // `PreUpdate`, beside the physics sync; a frame late is fine, since the cache epoch in each
        // body's spec rebuilds it.
        app.add_system(Stage::PreUpdate, fill_collider_meshes);
    }

    fn name(&self) -> &str {
        "ColliderMeshPlugin"
    }
}

/// What a collider still needs from a GUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Wanted {
    guid: Guid,
    /// Whether anyone wants the hull — reducing costs 33 ms on 76k vertices, wasted on
    /// triangle-only colliders.
    hull: bool,
}

/// Loads the mesh behind each unanswered collider and reduces wanted hulls. Once per GUID, failures
/// kept, so a hundred crates sharing a mesh load once.
pub fn fill_collider_meshes(resources: &mut Resources) {
    for want in unanswered(resources) {
        if !answered(resources, want.guid) {
            let mesh = load_mesh(resources, want.guid);
            let Some(mut cache) = resources.remove::<ColliderMeshCache>() else {
                return;
            };
            match mesh {
                Some(mesh) => cache.insert(want.guid, mesh),
                None => cache.fail(want.guid),
            }
            resources.insert(cache);
        }

        if want.hull {
            reduce_hull(resources, want.guid);
        }
    }
}

/// Replaces a cloud with its convex hull once — 76 038 points become 387, with qhull's faces, so
/// body builds skip qhull.
fn reduce_hull(resources: &mut Resources, guid: Guid) {
    let Some(cache) = resources.get::<ColliderMeshCache>() else {
        return;
    };
    if !cache.awaits_hull(guid) {
        return;
    }
    let Some((points, faces)) = cache.get(guid).and_then(|mesh| hull_of(&mesh.vertices)) else {
        // A cloud with no volume. `shape_builder` refuses it by name when
        // the body is built, which is where the author can act on it.
        return;
    };
    if let Some(cache) = resources.get_mut::<ColliderMeshCache>() {
        cache.insert_hull(guid, ConvexPart { points, faces });
    }
}

fn answered(resources: &Resources, guid: Guid) -> bool {
    resources
        .get::<ColliderMeshCache>()
        .is_some_and(|cache| cache.answered(guid))
}

/// What mesh-derived colliders still need.
fn unanswered(resources: &Resources) -> Vec<Wanted> {
    let Some(cache) = resources.get::<ColliderMeshCache>() else {
        return Vec::new();
    };
    let Some(colliders) = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<Collider>())
    else {
        return Vec::new();
    };

    let mut wanted: Vec<Wanted> = Vec::new();
    for (_, collider) in colliders.iter() {
        if !is_mesh_derived(collider.shape) {
            continue;
        }
        let Some(guid) = collider.mesh else { continue };
        let hull = collider.shape == SHAPE_CONVEX_HULL;
        if cache.answered(guid) && !(hull && cache.awaits_hull(guid)) {
            continue;
        }
        match wanted.iter_mut().find(|want| want.guid == guid) {
            // Two colliders on one mesh, one wanting a hull: the hull is
            // wanted. An `||` rather than the first answer seen.
            Some(want) => want.hull |= hull,
            None => wanted.push(Wanted { guid, hull }),
        }
    }
    // Component storage is a hash map, so the same scene asks in a
    // different order each run — and the cache's epoch is what bodies
    // rebuild on. Sorted, two runs agree.
    wanted.sort_unstable_by_key(|want| want.guid.as_uuid().as_u128());
    wanted
}

/// The mesh behind a GUID, or `None`, warning why: otherwise the body silently does not collide.
fn load_mesh(resources: &mut Resources, guid: Guid) -> Option<ColliderMesh> {
    let path = path_of(resources, guid)?;
    let bytes = read_bytes(resources, &path, guid)?;
    let base = path.parent();

    let mesh = match baked_kind(&path) {
        // Baked pieces: each primitive is one convex hull, and its
        // triangles are the engine's own claim that it is. Trusted, not
        // checked — see `ConvexPart`.
        Some(COLLISION_PARTS) => parse_mesh_parts(&bytes, base)
            .map(|parts| ColliderMesh {
                parts: parts
                    .into_iter()
                    .map(|(points, faces)| ConvexPart { points, faces })
                    .collect(),
                ..Default::default()
            })
            .map_err(|error| error.to_string()),
        // A baked hull is one convex piece, and the file is already it.
        // Hulling it again gave back the same 226 points at 162 µs a
        // build, which was the whole cost the bake existed to remove.
        Some(COLLISION_HULL) => parse_mesh_parts(&bytes, base)
            .map(|parts| {
                let (points, faces) = parts.into_iter().next().unwrap_or_default();
                ColliderMesh {
                    vertices: points.clone(),
                    hull: ConvexPart { points, faces },
                    ..Default::default()
                }
            })
            .map_err(|error| error.to_string()),
        _ => parse_mesh_bytes_full(&bytes, 1.0, base)
            .map(|mesh| ColliderMesh {
                vertices: mesh
                    .vertices
                    .iter()
                    .map(|vertex| glam::Vec3::from(vertex.position))
                    .collect(),
                indices: mesh
                    .indices
                    .chunks_exact(3)
                    .map(|tri| [tri[0], tri[1], tri[2]])
                    .collect(),
                ..Default::default()
            })
            .map_err(|error| error.to_string()),
    };

    match mesh {
        Ok(mesh) if !mesh.is_empty() => Some(mesh),
        Ok(_) => {
            warn(guid, "a collider names a mesh with no geometry in it");
            None
        }
        Err(error) => {
            tracing::warn!(
                target: "kooch::collider_mesh",
                guid = %guid,
                %error,
                "a collider names a mesh that will not parse, so its body will not collide",
            );
            None
        }
    }
}

fn path_of(resources: &Resources, guid: Guid) -> Option<PathBuf> {
    let db = resources.get::<AssetDatabase>()?;
    match db.entry(guid) {
        Some(entry) => Some(entry.path.clone()),
        None => {
            warn(
                guid,
                "a collider names a mesh the asset database does not know",
            );
            None
        }
    }
}

fn read_bytes(resources: &mut Resources, path: &Path, guid: Guid) -> Option<Vec<u8>> {
    let mut server = resources.remove::<AssetServer>()?;
    let bytes = server.read_bytes(path);
    resources.insert(server);
    match bytes {
        Ok(bytes) => Some(bytes),
        Err(error) => {
            tracing::warn!(
                target: "kooch::collider_mesh",
                guid = %guid,
                %error,
                "a collider names a mesh that will not read, so its body will not collide",
            );
            None
        }
    }
}

/// What the sidecar says this asset was baked as — the line between trusting topology and hulling.
/// Only the engine's bake and the editor's button write it; missing reads as an ordinary mesh.
fn baked_kind(path: &Path) -> Option<&'static str> {
    let value = asset_meta::read_meta(path)
        .ok()?
        .import?
        .get(COLLISION_KEY)
        .and_then(|value| value.as_str().map(str::to_owned))?;
    match value.as_str() {
        COLLISION_PARTS => Some(COLLISION_PARTS),
        COLLISION_HULL => Some(COLLISION_HULL),
        // `COLLISION_MESH` lands here with everything else: it is an
        // ordinary triangle mesh and takes the ordinary path.
        _ => None,
    }
}

fn warn(guid: Guid, message: &'static str) {
    tracing::warn!(target: "kooch::collider_mesh", guid = %guid, "{message}");
}

#[cfg(test)]
mod tests;
