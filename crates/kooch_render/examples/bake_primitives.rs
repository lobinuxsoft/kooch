//! Bakes the canonical primitives into `assets/meshes/primitives/`.
//!
//! Run from the engine root:
//!
//! ```text
//! cargo run -p kooch_render --example bake_primitives
//! ```
//!
//! # Why a command and not a test
//!
//! A test that writes into the repository is a test that fails on a
//! read-only checkout and rewrites tracked files as a side effect of
//! `cargo test`. Generation is a deliberate act; the *test* that the
//! generation is correct lives in `mesh::primitives` and
//! `mesh::export`, and needs no files at all.
//!
//! # Why the `.meta` sidecars are written here too
//!
//! The `AssetServer` mints a GUID on first import and writes the sidecar
//! itself — which would give every machine a different GUID for the same
//! cube, and a scene authored on one would not load on another. So the
//! GUIDs are fixed here, committed alongside the `.glb`, and re-running
//! this command keeps them: an existing sidecar is never overwritten.

use std::fs;
use std::path::{Path, PathBuf};

use kooch_render::mesh::{Primitive, to_glb};

/// Where the baked primitives live, relative to the engine root.
const OUT_DIR: &str = "assets/meshes/primitives";

/// The asset type the meshlet loader claims, so the sidecar matches what
/// an imported `.glb` would have produced.
const ASSET_TYPE: &str = "kooch_render::meshlet::asset::MeshletMesh";

/// Fixed GUIDs, one per canonical primitive, in `Primitive::CANONICAL`
/// order. Hard-coded so they survive regeneration — a churning GUID
/// breaks every scene that referenced the old one.
const GUIDS: [&str; 6] = [
    "0b1ec7a0-0000-4000-8000-000000000001", // cube
    "0b1ec7a0-0000-4000-8000-000000000002", // sphere
    "0b1ec7a0-0000-4000-8000-000000000003", // capsule
    "0b1ec7a0-0000-4000-8000-000000000004", // cylinder
    "0b1ec7a0-0000-4000-8000-000000000005", // cone
    "0b1ec7a0-0000-4000-8000-000000000006", // plane
];

fn main() {
    let root = engine_root();
    let out = root.join(OUT_DIR);
    if let Err(e) = fs::create_dir_all(&out) {
        eprintln!("cannot create {}: {e}", out.display());
        std::process::exit(1);
    }

    for (i, (name, primitive)) in Primitive::CANONICAL.into_iter().enumerate() {
        let mesh = primitive.build();
        let bytes = match to_glb(&mesh, name) {
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

        let meta = out.join(format!("{name}.glb.meta"));
        let existing = fs::read_to_string(&meta).ok();
        match existing {
            // Never overwrite: whatever GUID a scene already references
            // has to keep resolving, even if `GUIDS` drifted.
            Some(_) => println!(
                "{name:9} {:6} tris  {} (guid kept)",
                mesh.index_count() / 3,
                glb.display()
            ),
            None => {
                let contents = format!("guid = \"{}\"\nasset_type = \"{ASSET_TYPE}\"\n", GUIDS[i]);
                if let Err(e) = fs::write(&meta, contents) {
                    eprintln!("{name}: cannot write {}: {e}", meta.display());
                    std::process::exit(1);
                }
                println!(
                    "{name:9} {:6} tris  {} (guid minted)",
                    mesh.index_count() / 3,
                    glb.display()
                );
            }
        }
    }

    println!("\nbaked {} primitives into {}", GUIDS.len(), out.display());
}

/// Resolves the engine root: the manifest directory's grandparent, so the
/// command works from anywhere rather than only from the repo root.
fn engine_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("kooch_render is not two levels below the engine root")
        .to_path_buf()
}
