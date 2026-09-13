//! Baking a collision mesh out of a render mesh.

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

use crate::actions::BakeKind;
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
pub(super) fn handle_bake_collider(
    resources: &mut Resources,
    source: Guid,
    kind: BakeKind,
    max_faces: u32,
) {
    if kind == BakeKind::Mesh && max_faces == 0 {
        tracing::warn!("Create simplified mesh: set a face budget, or the result is a copy");
        return;
    }
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

    let parts: Vec<Mesh> = match kind {
        // Not hulled at all: the triangles are the product, decimated.
        BakeKind::Mesh => vec![decimate(&mesh, max_faces, source)],
        BakeKind::Parts => decompose(&positions, &triangles)
            .iter()
            .filter_map(|points| hull_mesh(points, max_faces))
            .collect(),
        BakeKind::Hull => hull_mesh(&positions, max_faces).into_iter().collect(),
    };
    let parts: Vec<Mesh> = parts
        .into_iter()
        .filter(|m| !m.indices.is_empty())
        .collect();
    if parts.is_empty() {
        tracing::warn!(
            guid = %source,
            "Create collision mesh: the source has no volume to build a hull from",
        );
        return;
    }

    let suffix = kind.tag();
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

/// The mesh's own triangles, decimated to a budget.
fn decimate(mesh: &Mesh, max_faces: u32, source: Guid) -> Mesh {
    let (smaller, error) = simplify(mesh, SimplifyTarget::Triangles(max_faces));
    tracing::info!(
        guid = %source,
        from = mesh.indices.len() / 3,
        to = smaller.indices.len() / 3,
        deviation = error,
        "simplified collision mesh; the deviation is how far the surface moved",
    );
    smaller
}

/// The convex hull of a point cloud, as a mesh, optionally decimated.
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
fn hash_of(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests;
