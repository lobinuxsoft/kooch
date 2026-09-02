//! The engine's committed collision meshes, checked as files.
//!
//! Same job as `baked_primitives`: the unit tests cover the export in
//! memory, and these cover the part that only fails on someone else's
//! machine — a file that was never committed, a sidecar whose GUID
//! drifted, or a decomposition that came back as one merged blob.

use std::path::{Path, PathBuf};

use kooch_render::mesh::parse_mesh_parts;

fn engine_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("kooch_render is not two levels below the engine root")
        .to_path_buf()
}

fn collision_dir() -> PathBuf {
    engine_root().join("assets/meshes/collision")
}

/// Every bake `examples/bake_colliders.rs` produces is on disk with a
/// sidecar beside it.
///
/// A vendored engine copies `assets/meshes` whole, so a missing file here
/// is a project whose collider silently never resolves.
#[test]
fn every_bake_is_committed() {
    for name in [
        "suzanne_hull",
        "suzanne_parts",
        "dragon_hull",
        "dragon_parts",
    ] {
        let glb = collision_dir().join(format!("{name}.glb"));
        assert!(glb.exists(), "{} was never committed", glb.display());
        let meta = collision_dir().join(format!("{name}.glb.meta"));
        assert!(
            meta.exists(),
            "{} has no sidecar, so it has no GUID",
            meta.display()
        );
    }
}

/// A hull is one convex piece, and a decomposition is several.
///
/// The assertion that catches the failure nothing else would: an exporter
/// that merged the pieces would produce a file that loads, draws, and
/// collides as the concave solid the decomposition exists to avoid.
#[test]
fn a_decomposition_stays_in_pieces() {
    let pieces = |name: &str| {
        let bytes = std::fs::read(collision_dir().join(format!("{name}.glb")))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        parse_mesh_parts(&bytes, None).unwrap_or_else(|e| panic!("{name}: {e}"))
    };

    assert_eq!(pieces("suzanne_hull").len(), 1);
    assert_eq!(pieces("dragon_hull").len(), 1);
    assert!(pieces("suzanne_parts").len() > 1, "the pieces were merged");
    assert!(pieces("dragon_parts").len() > 1, "the pieces were merged");
}

/// Each piece has to be something a solver can build a hull from.
///
/// Four points is a tetrahedron, the smallest thing that encloses a
/// volume; anything less is a collider that cannot be hit.
#[test]
fn every_piece_has_volume() {
    for name in [
        "suzanne_hull",
        "suzanne_parts",
        "dragon_hull",
        "dragon_parts",
    ] {
        let bytes = std::fs::read(collision_dir().join(format!("{name}.glb")))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        for (index, piece) in parse_mesh_parts(&bytes, None)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .iter()
            .enumerate()
        {
            assert!(
                piece.len() >= 4,
                "{name} piece {index} has {} points",
                piece.len()
            );
        }
    }
}

/// A collider is only worth baking if it is cheaper than the mesh it came
/// from — that is the entire argument for the file existing.
#[test]
fn a_hull_is_smaller_than_its_source() {
    for (source, hull) in [("suzanne", "suzanne_hull"), ("dragon", "dragon_hull")] {
        let source_bytes = std::fs::metadata(
            engine_root()
                .join("assets/meshes")
                .join(format!("{source}.glb")),
        )
        .expect("source mesh")
        .len();
        let hull_bytes = std::fs::metadata(collision_dir().join(format!("{hull}.glb")))
            .expect("baked hull")
            .len();
        assert!(
            hull_bytes < source_bytes,
            "{hull} is {hull_bytes} bytes against {source}'s {source_bytes}",
        );
    }
}
