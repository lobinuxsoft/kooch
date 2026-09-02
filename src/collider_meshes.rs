//! [`ColliderMeshPlugin`] — the system that fills
//! [`ColliderMeshCache`](kooch_physics::ColliderMeshCache).
//!
//! # Why it lives in the facade
//!
//! It is the one job that needs both halves: `Collider.mesh` is a GUID,
//! resolving one means the asset database, and the asset database means
//! `kooch_render`. Physics must not depend on the renderer — that would
//! tie [`PhysicsBackend`] to wgpu and make the trait unswappable — and
//! the renderer has no business knowing what a collider is. This crate
//! already sees both, and is the only one that should.
//!
//! # It reads the file, not the render asset
//!
//! The obvious implementation asks the `AssetServer` for a `MeshletMesh`
//! and decodes its LOD 0 back into triangles. It is also nearly three
//! seconds of wasted work for a 76k-vertex mesh, measured in debug:
//! building the meshlet LOD chain costs 2.9 s, and every one of the 4832
//! meshlets it produces is thrown away by the decode on the next line.
//! Parsing the `.glb` for positions and indices is 36 ms.
//!
//! In a windowed game the renderer builds those meshlets anyway and the
//! collider would ride along for free. The editor's host is the case that
//! matters: it simulates and draws nothing, so the whole chain is waste —
//! as is a collision proxy that is never rendered.
//!
//! [`PhysicsBackend`]: kooch_physics::PhysicsBackend

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

/// The `[import]` key a baked collision asset carries, and the value that
/// means "each primitive is one convex piece".
///
/// In the sidecar rather than inferred from the primitive count: an
/// ordinary artist mesh is often several primitives, one per material,
/// and reading those as convex pieces would silently turn one prop into
/// a handful of overlapping hulls.
pub const COLLISION_KEY: &str = "collision";
pub const COLLISION_PARTS: &str = "parts";
pub const COLLISION_HULL: &str = "hull";

/// Resolves the meshes mesh-derived colliders name.
pub struct ColliderMeshPlugin;

impl Plugin for ColliderMeshPlugin {
    fn build(&self, app: &mut App) {
        // `PreUpdate`, alongside the physics sync it feeds. Landing a
        // frame late is not a race: the cache's epoch is in every body's
        // spec, so a mesh arriving after the body was authored retires it
        // and the next frame rebuilds with the geometry.
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
    /// Whether anything asks for the convex hull of this mesh.
    ///
    /// Tracked separately because reducing one costs 33 ms on a 76k
    /// mesh, and a collider that only ever wants the triangles should
    /// not pay for a hull it will not use.
    hull: bool,
}

/// Loads the mesh behind every mesh-derived collider that has no answer,
/// and reduces the hull of every one that wants it.
///
/// Asked once per GUID, not once per frame: an answer — including a
/// failure — is kept, so a scene where a hundred crates share one
/// collision mesh does one load.
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

/// Replaces a mesh's point cloud with its convex hull, once.
///
/// 76 038 points become 387, and the faces come back with them — so the
/// backend builds the polyhedron straight from qhull's own output rather
/// than asking qhull for it again on every body build.
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

/// The mesh behind a GUID, or `None` when it will not resolve.
///
/// Says why at `warn` — a collider that never appears is otherwise a body
/// that silently is not there, and the GUID is the only clue.
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

/// What this asset's sidecar says it was baked as, if anything.
///
/// The line between "trust this topology" and "hull these points". Only
/// the engine's own bake and the editor's button write this key, so a
/// mesh an artist authored — or one edited by hand after the marker was
/// removed — is hulled like any other.
///
/// A missing or unreadable sidecar reads as "ordinary mesh", which is the
/// answer that keeps every asset authored before this existed working.
fn baked_kind(path: &Path) -> Option<&'static str> {
    let value = asset_meta::read_meta(path)
        .ok()?
        .import?
        .get(COLLISION_KEY)
        .and_then(|value| value.as_str().map(str::to_owned))?;
    match value.as_str() {
        COLLISION_PARTS => Some(COLLISION_PARTS),
        COLLISION_HULL => Some(COLLISION_HULL),
        _ => None,
    }
}

fn warn(guid: Guid, message: &'static str) {
    tracing::warn!(target: "kooch::collider_mesh", guid = %guid, "{message}");
}

#[cfg(test)]
mod tests;
