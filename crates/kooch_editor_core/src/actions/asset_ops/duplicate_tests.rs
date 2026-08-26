use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A copy is a distinct asset, so it needs an id of its own — sharing
/// the source's would make two files claim one identity and whichever
/// registered last would win.
#[test]
fn a_copy_gets_a_fresh_id_carrying_the_source_type() {
    let dir = scratch("kooch_dup_identity");
    let source = dir.join("Enemy.prefab");
    let dest = dir.join("Enemy_1.prefab");
    std::fs::write(&source, "()").unwrap();
    std::fs::write(&dest, "()").unwrap();
    let original = kooch_core::asset_meta::AssetMeta::with_type("test::Thing");
    kooch_core::asset_meta::write_meta(&source, &original).unwrap();

    let mut resources = kooch_core::resource::Resources::new();
    duplicate_identity(&mut resources, &source, &dest);

    let copy = kooch_core::asset_meta::read_meta(&dest).expect("the copy has an identity");
    assert_ne!(copy.guid, original.guid, "the copy aliased the original");
    assert_eq!(
        copy.asset_type,
        Some("test::Thing".to_owned()),
        "the copy holds the same kind of thing as its source",
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A plain file copied in the browser is still a plain file. Inventing
/// an identity for it would list it in asset pickers as a typeless
/// entry nothing can load.
#[test]
fn copying_something_that_is_not_an_asset_stays_not_an_asset() {
    let dir = scratch("kooch_dup_plain");
    let source = dir.join("notes.txt");
    let dest = dir.join("notes_1.txt");
    std::fs::write(&source, "hello").unwrap();
    std::fs::write(&dest, "hello").unwrap();

    let mut resources = kooch_core::resource::Resources::new();
    duplicate_identity(&mut resources, &source, &dest);

    assert!(kooch_core::asset_meta::read_meta(&dest).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- which folder the IDE opens as its workspace -------------------

use crate::project_state::{ActiveProject, ProjectState};
use kooch_core::resource::Resources;

fn resources_with_roots(project: &str, engine: &str) -> Resources {
    let mut state = ProjectState::new();
    state.active_project = Some(ActiveProject {
        manifest: crate::project::ProjectManifest::new("test"),
        root_path: PathBuf::from(project),
    });
    state.engine_root = Some(PathBuf::from(engine));
    let mut resources = Resources::new();
    resources.insert(state);
    resources
}

/// The bug this replaced: all three call sites passed the asset
/// browser's root, so the IDE opened `<project>/assets` — a workspace
/// with no `Cargo.toml` and no `src/`.
#[test]
fn a_project_asset_opens_the_crate_root_not_the_assets_folder() {
    let resources = resources_with_roots("/proj", "/engine");

    let workspace = workspace_for(&resources, Path::new("/proj/assets/Player.prefab"));

    assert_eq!(
        workspace,
        Some(PathBuf::from("/proj")),
        "the workspace must be the crate root, or there is no source to edit"
    );
}

#[test]
fn a_source_file_opens_the_same_root_as_an_asset() {
    let resources = resources_with_roots("/proj", "/engine");
    assert_eq!(
        workspace_for(&resources, Path::new("/proj/src/player.rs")),
        workspace_for(&resources, Path::new("/proj/assets/Player.prefab")),
    );
}

/// Engine assets are read-only, and opening them beside the engine's
/// own source is what makes them worth looking at.
#[test]
fn an_engine_asset_opens_the_engine_root() {
    let resources = resources_with_roots("/proj", "/engine");
    assert_eq!(
        workspace_for(&resources, Path::new("/engine/assets/meshes/cube.glb")),
        Some(PathBuf::from("/engine")),
    );
}

#[test]
fn a_file_under_neither_root_claims_no_workspace() {
    let resources = resources_with_roots("/proj", "/engine");
    assert_eq!(workspace_for(&resources, Path::new("/tmp/stray.ron")), None);
}

#[test]
fn no_project_open_claims_no_workspace() {
    let mut resources = Resources::new();
    resources.insert(ProjectState::new());
    assert_eq!(
        workspace_for(&resources, Path::new("/proj/assets/a.ron")),
        None
    );
}
