//! Bakes the engine's own collision meshes into `assets/meshes/collision/`.
//!
//! Run from anywhere:
//!
//! ```text
//! cargo run --features physics --example bake_colliders
//! ```
//!
//! # Why the engine ships these
//!
//! A project that points a `Collider` at an engine mesh would otherwise
//! derive the shape itself, every session. The hull is cheap enough not
//! to care; the convex decomposition is not — 1.35 s for Suzanne and
//! 2.58 s for the dragon in a debug build, which is what a project's
//! host runs. The engine's meshes are the engine's to bake, the same way
//! the primitives are (`kooch_render --example bake_primitives`).
//!
//! # Why not the primitives
//!
//! A cube already has an exact collision shape, and so does every other
//! canonical primitive. Their hulls would be worse copies of shapes the
//! `Collider` dropdown already offers.
//!
//! # Why the GUIDs are fixed
//!
//! The same reason `bake_primitives` fixes its own: the `AssetServer`
//! mints one on first import, so every machine would get a different
//! GUID for the same hull and a scene authored on one would not load on
//! another. An existing sidecar is never overwritten.

use std::fs;
use std::path::{Path, PathBuf};

use kooch_physics::{decompose, hull_of};
use kooch_render::mesh::{Mesh, parse_mesh_bytes_full, to_glb, to_glb_parts};

/// Where the baked colliders live, relative to the engine root.
///
/// Under `meshes/`, which travels whole into a vendored engine — see
/// `engine_vendor::COPY_ASSETS`.
const OUT_DIR: &str = "assets/meshes/collision";

/// The type the sidecar claims, so a baked hull appears in the same
/// picker `Collider.mesh` filters by.
const ASSET_TYPE: &str = "kooch_render::meshlet::asset::MeshletMesh";

/// The engine meshes worth baking, and the fixed GUID of each product.
///
/// Hard-coded so they survive regeneration — a churning GUID breaks every
/// scene that referenced the old one.
const BAKES: [(&str, &str, &str); 4] = [
    ("suzanne", "hull", "c0111de0-0000-4000-8000-000000000001"),
    ("suzanne", "parts", "c0111de0-0000-4000-8000-000000000002"),
    ("dragon", "hull", "c0111de0-0000-4000-8000-000000000003"),
    ("dragon", "parts", "c0111de0-0000-4000-8000-000000000004"),
];

fn main() {
    let root = engine_root();
    let out = root.join(OUT_DIR);
    if let Err(e) = fs::create_dir_all(&out) {
        eprintln!("cannot create {}: {e}", out.display());
        std::process::exit(1);
    }

    for (stem, kind, guid) in BAKES {
        let source = root.join("assets/meshes").join(format!("{stem}.glb"));
        let (positions, triangles) = read_source(&source);

        let started = std::time::Instant::now();
        let pieces = match kind {
            "parts" => decompose(&positions, &triangles),
            _ => vec![positions],
        };
        let parts: Vec<Mesh> = pieces
            .iter()
            .filter_map(|points| {
                let (hull, faces) = hull_of(points)?;
                Some(Mesh::from_triangles(&hull, &faces))
            })
            .collect();
        if parts.is_empty() {
            eprintln!("{stem} {kind}: no volume to build a hull from");
            std::process::exit(1);
        }

        let name = format!("{stem}_{kind}");
        let named: Vec<(&Mesh, String)> = parts
            .iter()
            .enumerate()
            .map(|(i, mesh)| (mesh, format!("{name}_{i}")))
            .collect();
        let borrowed: Vec<(&Mesh, &str)> = named.iter().map(|(m, n)| (*m, n.as_str())).collect();
        let bytes = match parts.len() {
            1 => to_glb(&parts[0], &name),
            _ => to_glb_parts(&borrowed),
        };
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("{name}: export failed: {e}");
                std::process::exit(1);
            }
        };

        let glb = out.join(format!("{name}.glb"));
        if let Err(e) = fs::write(&glb, &bytes) {
            eprintln!("{name}: cannot write {}: {e}", glb.display());
            std::process::exit(1);
        }
        let faces: usize = parts.iter().map(|m| m.indices.len() / 3).sum();
        println!(
            "{name:16} {:2} pieces {faces:5} faces  {:>8.0?}  {}",
            parts.len(),
            started.elapsed(),
            write_meta(&out, &name, guid),
        );
    }

    println!(
        "\nbaked {} collision meshes into {}",
        BAKES.len(),
        out.display()
    );
}

/// Writes the sidecar unless one is already there, and says which.
///
/// No `source_guid` or `source_hash`: those exist so the *editor* can
/// flag a project's bake as stale, and the engine's own meshes change
/// only when this command is re-run beside them in the same commit.
fn write_meta(out: &Path, name: &str, guid: &str) -> &'static str {
    let meta = out.join(format!("{name}.glb.meta"));
    if meta.exists() {
        // Never overwrite: whatever GUID a scene already references has
        // to keep resolving, even if `BAKES` drifted.
        return "guid kept";
    }
    let contents = format!(
        "guid = \"{guid}\"\nasset_type = \"{ASSET_TYPE}\"\n\n[import]\ncollision = \"{kind}\"\n",
        kind = name.rsplit('_').next().unwrap_or("hull")
    );
    if let Err(e) = fs::write(&meta, contents) {
        eprintln!("{name}: cannot write {}: {e}", meta.display());
        std::process::exit(1);
    }
    "guid minted"
}

/// The source mesh's positions and triangles.
fn read_source(path: &Path) -> (Vec<glam::Vec3>, Vec<[u32; 3]>) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("cannot read {}: {e}", path.display());
            std::process::exit(1);
        }
    };
    let mesh = match parse_mesh_bytes_full(&bytes, 1.0, path.parent()) {
        Ok(mesh) => mesh,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            std::process::exit(1);
        }
    };
    (
        mesh.vertices
            .iter()
            .map(|v| glam::Vec3::from(v.position))
            .collect(),
        mesh.indices
            .chunks_exact(3)
            .map(|t| [t[0], t[1], t[2]])
            .collect(),
    )
}

/// The engine root: this manifest's directory.
fn engine_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}
