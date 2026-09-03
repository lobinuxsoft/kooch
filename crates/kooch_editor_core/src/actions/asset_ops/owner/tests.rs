use std::path::Path;

use super::engine_owned;

fn owned(path: &str) -> bool {
    engine_owned(
        Some(Path::new("/proj")),
        Some(Path::new("/engine")),
        Path::new(path),
    )
}

#[test]
fn an_engine_asset_is_theirs() {
    assert!(owned("/engine/assets/materials/default.ron"));
}

#[test]
fn a_project_asset_is_ours() {
    assert!(!owned("/proj/assets/Player.prefab"));
}

#[test]
fn a_stray_path_is_neither() {
    assert!(!owned("/tmp/stray.ron"));
}

/// The editor built from a project points both roots at it. Engine-first
/// would turn every asset in that project read-only.
#[test]
fn the_project_wins_a_shared_root() {
    let root = Some(Path::new("/proj"));
    assert!(!engine_owned(root, root, Path::new("/proj/assets/a.ron")));
}

/// A sibling starting with the same letters is a different tree —
/// `starts_with` compares components, and this is what proves it.
#[test]
fn a_lookalike_sibling_is_neither() {
    assert!(!owned("/engine_backup/assets/a.ron"));
}

#[test]
fn no_engine_root_owns_nothing() {
    assert!(!engine_owned(
        Some(Path::new("/proj")),
        None,
        Path::new("/engine/assets/a.ron"),
    ));
}
