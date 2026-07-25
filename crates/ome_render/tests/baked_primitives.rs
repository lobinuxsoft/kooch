//! The baked primitives on disk, checked against the generator that
//! produced them.
//!
//! The unit tests cover generation and export in memory. These cover the
//! part that only fails on someone else's machine: a file that was never
//! committed, a sidecar whose GUID drifted, or a menu entry pointing at a
//! path that does not exist.

use std::path::{Path, PathBuf};

use ome_render::mesh::{Primitive, parse_mesh_bytes};

/// The engine root, two levels above this crate's manifest.
fn engine_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("ome_render is not two levels below the engine root")
        .to_path_buf()
}

fn primitive_dir() -> PathBuf {
    engine_root().join("assets/meshes/primitives")
}

/// Every canonical primitive has a committed `.glb`, and it is the mesh
/// the generator produces today.
///
/// This is the test that fails when someone edits a generator and forgets
/// to re-bake — the editor would keep spawning the old geometry, with
/// nothing to indicate the code and the asset had diverged.
#[test]
fn every_baked_primitive_matches_its_generator() {
    for (name, primitive) in Primitive::CANONICAL {
        let path = primitive_dir().join(format!("{name}.glb"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "{name}: {} is missing ({e}). Re-run: \
                 cargo run -p ome_render --example bake_primitives",
                path.display()
            )
        });

        let baked = parse_mesh_bytes(&bytes).unwrap_or_else(|e| {
            panic!("{name}: the engine cannot read its own baked asset: {e:?}")
        });
        let generated = primitive.build();

        assert_eq!(
            baked.vertex_count(),
            generated.vertex_count(),
            "{name}: the baked asset is stale — re-run the bake_primitives example"
        );
        assert_eq!(
            baked.index_count(),
            generated.index_count(),
            "{name}: the baked asset is stale — re-run the bake_primitives example"
        );
        assert!(
            baked.aabb.min.abs_diff_eq(generated.aabb.min, 1e-4)
                && baked.aabb.max.abs_diff_eq(generated.aabb.max, 1e-4),
            "{name}: baked bounds {:?}..{:?} differ from generated {:?}..{:?}",
            baked.aabb.min,
            baked.aabb.max,
            generated.aabb.min,
            generated.aabb.max
        );
    }
}

/// Each primitive has a `.meta` with a GUID, and every GUID is distinct.
///
/// The sidecars are committed precisely so the GUIDs do not get minted
/// per machine: a scene authored on one checkout has to load on another.
/// Two primitives sharing a GUID would make one silently render as the
/// other.
#[test]
fn every_primitive_has_a_distinct_committed_guid() {
    let mut guids: Vec<(String, String)> = Vec::new();

    for (name, _) in Primitive::CANONICAL {
        let path = primitive_dir().join(format!("{name}.glb.meta"));
        let contents = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{name}: {} is missing ({e})", path.display()));

        let guid = contents
            .lines()
            .find_map(|l| l.strip_prefix("guid = "))
            .map(|v| v.trim().trim_matches('"').to_owned())
            .unwrap_or_else(|| panic!("{name}: sidecar has no guid:\n{contents}"));

        assert!(
            !guid.is_empty() && guid.len() == 36,
            "{name}: implausible guid {guid:?}"
        );
        assert!(
            contents.contains("asset_type"),
            "{name}: sidecar has no asset_type, the loader will not claim it"
        );
        guids.push((name.to_owned(), guid));
    }

    for (i, (name, guid)) in guids.iter().enumerate() {
        if let Some((other, _)) = guids[..i].iter().find(|(_, g)| g == guid) {
            panic!("{name} and {other} share the guid {guid}");
        }
    }
}

/// The paths the editor's spawn menu builds resolve to real files.
///
/// The menu constructs `meshes/primitives/{name}.glb` from the same
/// `CANONICAL` list, so this is what catches a rename that updates the
/// list but not the assets.
#[test]
fn the_spawn_menu_paths_resolve() {
    let assets = engine_root().join("assets");
    for (name, _) in Primitive::CANONICAL {
        let relative = format!("meshes/primitives/{name}.glb");
        assert!(
            assets.join(&relative).is_file(),
            "the spawn menu points at {relative}, which does not exist"
        );
    }
}
