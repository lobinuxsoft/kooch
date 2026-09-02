//! Baking a collision mesh out of a render mesh.
//!
//! # Why this is a file and not a cache
//!
//! Both halves of the answer are measured, in debug, on the engine's own
//! meshes. A convex hull is 33 ms and its result is cached at runtime, so
//! baking one saves little on its own. A convex *decomposition* is 1.35 s
//! for Suzanne and 2.58 s for a 76k dragon, every time the body is
//! rebuilt — a scale drag re-runs it. That is not something to cache; it
//! is something to compute once and keep.
//!
//! The file buys two more things a cache cannot: an artist can open it
//! and see what the solver will actually collide against, and it can be
//! simplified below the exact hull, which nothing at runtime is allowed
//! to do on its own.
//!
//! # The derived asset knows where it came from
//!
//! A baked collider is the classic silent-staleness trap: change the
//! source, and the hull keeps its own GUID, nothing fails, and the prop
//! collides with the shape it had last week. So the sidecar records the
//! source GUID and a hash of the source bytes. Whoever displays it can
//! say "this is behind"; nothing has to guess.

use std::path::{Path, PathBuf};

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::{AssetServer, asset_written};
use kooch_core::asset_meta::{self, AssetMeta};
use kooch_core::resource::Resources;
use kooch_physics::{decompose, hull_of};
use kooch_render::mesh::{
    Mesh, SimplifyTarget, parse_mesh_bytes_full, simplify, to_glb, to_glb_parts,
};

use crate::project_state::ProjectState;

/// Where baked colliders land, under the project's asset root.
const OUT_DIR: &str = "collision";

/// The type the sidecar claims, so a baked hull shows up in the same
/// picker the source mesh does — which is the picker `Collider.mesh`
/// filters by.
const ASSET_TYPE: &str = "kooch_render::meshlet::asset::MeshletMesh";

/// Sidecar keys. `source` and `hash` are what make a stale bake
/// detectable instead of silent.
const KEY_KIND: &str = "collision";
const KEY_SOURCE: &str = "source_guid";
const KEY_HASH: &str = "source_hash";

/// Builds a collision mesh beside the project's assets.
///
/// `concave` picks the decomposition over the single hull. `max_faces`
/// caps each piece; zero means the exact hull, which is what qhull
/// already reduces to and is correct if dearer.
pub(super) fn handle_bake_collider(
    resources: &mut Resources,
    source: Guid,
    concave: bool,
    max_faces: u32,
) {
    let Some(out_dir) = project_collision_dir(resources) else {
        tracing::warn!(
            "Create collision mesh: no project is open, so there is nowhere to write it"
        );
        return;
    };
    let Some((source_path, bytes)) = read_source(resources, source) else {
        return;
    };

    let mesh = match parse_mesh_bytes_full(&bytes, 1.0, source_path.parent()) {
        Ok(mesh) => mesh,
        Err(error) => {
            tracing::warn!(guid = %source, %error, "Create collision mesh: the source will not parse");
            return;
        }
    };
    let positions: Vec<glam::Vec3> = mesh
        .vertices
        .iter()
        .map(|vertex| glam::Vec3::from(vertex.position))
        .collect();
    let triangles: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .map(|tri| [tri[0], tri[1], tri[2]])
        .collect();

    let pieces = match concave {
        true => decompose(&positions, &triangles),
        false => vec![positions],
    };
    let parts: Vec<Mesh> = pieces
        .iter()
        .filter_map(|points| hull_mesh(points, max_faces))
        .collect();
    if parts.is_empty() {
        tracing::warn!(
            guid = %source,
            "Create collision mesh: the source has no volume to build a hull from",
        );
        return;
    }

    let suffix = match concave {
        true => "parts",
        false => "hull",
    };
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mesh");
    let out = out_dir.join(format!("{stem}_{suffix}.glb"));

    let named: Vec<(&Mesh, String)> = parts
        .iter()
        .enumerate()
        .map(|(index, part)| (part, format!("{stem}_{suffix}_{index}")))
        .collect();
    let borrowed: Vec<(&Mesh, &str)> = named
        .iter()
        .map(|(mesh, name)| (*mesh, name.as_str()))
        .collect();

    let glb = match parts.len() {
        1 => to_glb(&parts[0], &format!("{stem}_{suffix}")),
        _ => to_glb_parts(&borrowed),
    };
    let glb = match glb {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(guid = %source, %error, "Create collision mesh: export failed");
            return;
        }
    };

    if let Err(error) = std::fs::create_dir_all(&out_dir) {
        tracing::warn!(path = %out_dir.display(), %error, "Create collision mesh: cannot create the folder");
        return;
    }
    if let Err(error) = std::fs::write(&out, &glb) {
        tracing::warn!(path = %out.display(), %error, "Create collision mesh: cannot write the file");
        return;
    }

    write_sidecar(&out, source, suffix, hash_of(&bytes));
    asset_written(&out, resources);
    tracing::info!(
        path = %out.display(),
        pieces = parts.len(),
        "collision mesh written; point the collider's mesh at it",
    );
}

/// The convex hull of a point cloud, as a mesh, optionally decimated.
///
/// Simplified *then re-hulled*: `meshopt` collapses edges and has no
/// reason to keep the result convex, and a collider that is nearly convex
/// is a collider with a dent the solver will find.
fn hull_mesh(points: &[glam::Vec3], max_faces: u32) -> Option<Mesh> {
    let (hull, triangles) = hull_of(points)?;
    let mesh = Mesh::from_triangles(&hull, &triangles);
    if max_faces == 0 || triangles.len() as u32 <= max_faces {
        return Some(mesh);
    }

    let (smaller, _) = simplify(&mesh, SimplifyTarget::Triangles(max_faces));
    let reduced: Vec<glam::Vec3> = smaller
        .vertices
        .iter()
        .map(|vertex| glam::Vec3::from(vertex.position))
        .collect();
    let (hull, triangles) = hull_of(&reduced)?;
    Some(Mesh::from_triangles(&hull, &triangles))
}

/// `<project>/assets/collision`.
///
/// The project, never the engine: the engine's assets are read-only, and
/// its own meshes get their colliders baked into the engine the way the
/// primitives are.
fn project_collision_dir(resources: &Resources) -> Option<PathBuf> {
    let state = resources.get::<ProjectState>()?;
    let root = state.active_project.as_ref()?.root_path.clone();
    Some(root.join("assets").join(OUT_DIR))
}

fn read_source(resources: &mut Resources, source: Guid) -> Option<(PathBuf, Vec<u8>)> {
    let path = resources
        .get::<AssetDatabase>()
        .and_then(|db| db.entry(source).map(|entry| entry.path.clone()))?;
    let mut server = resources.remove::<AssetServer>()?;
    let bytes = server.read_bytes(&path);
    resources.insert(server);
    match bytes {
        Ok(bytes) => Some((path, bytes)),
        Err(error) => {
            tracing::warn!(guid = %source, %error, "Create collision mesh: the source will not read");
            None
        }
    }
}

/// Writes the derived asset's sidecar, with the link back to its source.
fn write_sidecar(out: &Path, source: Guid, kind: &str, hash: u64) {
    // An existing sidecar keeps its GUID: re-baking has to leave every
    // collider that already points here pointing here.
    let mut meta = asset_meta::read_meta(out).unwrap_or_else(|_| AssetMeta::with_type(ASSET_TYPE));
    meta.asset_type = Some(ASSET_TYPE.to_owned());

    let mut import = meta.import.take().unwrap_or_default();
    import.insert(KEY_KIND.into(), kind.into());
    import.insert(KEY_SOURCE.into(), source.to_string().into());
    // As a string: TOML integers are signed 64-bit and a hash uses the
    // whole range, so half of them would not round-trip as numbers.
    import.insert(KEY_HASH.into(), format!("{hash:016x}").into());
    meta.import = Some(import);

    if let Err(error) = asset_meta::write_meta(out, &meta) {
        tracing::warn!(path = %out.display(), %error, "collision mesh written without a sidecar");
    }
}

/// A cheap fingerprint of the source bytes.
///
/// Not cryptographic and does not need to be: the question is "did this
/// file change", and an adversary editing your meshes has already won.
fn hash_of(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests;
