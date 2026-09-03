//! How far a delete is allowed to reach (#815).
//!
//! Project assets stay permanent — no dialog, no trash, no undo. The
//! engine's own are refused, because a scene referencing one holds no
//! copy of it: removing it breaks every project on the machine sharing
//! the install, not the one that is open.

use std::path::{Path, PathBuf};

use kooch_core::resource::Resources;

use super::{delete_asset, delete_folder};
use crate::project_state::{ActiveProject, ProjectState};

/// A project tree and an engine tree, both real on disk, with the
/// resources that say which is which.
fn roots(name: &str) -> (PathBuf, PathBuf, Resources) {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join("proj");
    let engine = dir.join("engine");
    std::fs::create_dir_all(project.join("assets")).unwrap();
    std::fs::create_dir_all(engine.join("assets/materials")).unwrap();

    let mut state = ProjectState::new();
    state.active_project = Some(ActiveProject {
        manifest: crate::project::ProjectManifest::new("test"),
        root_path: project.clone(),
    });
    state.engine_root = Some(engine.clone());
    let mut resources = Resources::new();
    resources.insert(state);
    (project, engine, resources)
}

fn written(path: &Path) -> PathBuf {
    std::fs::write(path, "()").unwrap();
    path.to_path_buf()
}

#[test]
fn an_engine_asset_survives_delete() {
    let (_, engine, mut resources) = roots("kooch_815_engine_asset");
    let asset = written(&engine.join("assets/materials/default.kooch_material.ron"));
    let meta = written(&super::meta_path(&asset));

    delete_asset(&mut resources, &asset);

    assert!(asset.exists(), "an engine asset was removed");
    assert!(meta.exists(), "an engine asset's identity was removed");
}

/// `remove_dir_all` is the call that makes this unrecoverable, so the
/// folder is tested with children rather than empty.
#[test]
fn an_engine_folder_survives_delete() {
    let (_, engine, mut resources) = roots("kooch_815_engine_folder");
    let folder = engine.join("assets/materials");
    written(&folder.join("default.kooch_material.ron"));

    delete_folder(&mut resources, &folder);

    assert!(folder.exists(), "the engine's materials folder was removed");
}

#[test]
fn a_project_asset_still_deletes() {
    let (project, _, mut resources) = roots("kooch_815_project_asset");
    let asset = written(&project.join("assets/Player.prefab"));

    delete_asset(&mut resources, &asset);

    assert!(!asset.exists(), "a project asset is the author's to delete");
}

#[test]
fn a_project_folder_still_deletes() {
    let (project, _, mut resources) = roots("kooch_815_project_folder");
    let folder = project.join("assets/props");
    std::fs::create_dir_all(&folder).unwrap();
    written(&folder.join("rock.prefab"));

    delete_folder(&mut resources, &folder);

    assert!(
        !folder.exists(),
        "a project folder is the author's to delete"
    );
}

/// 🔴 An editor built from a project resolves its engine root to that
/// same project. Asking the engine first would make every asset in it
/// undeletable — the guard would look like it worked, and nothing in the
/// project could be removed again.
#[test]
fn a_project_that_is_its_own_engine_deletes() {
    let (project, _, _) = roots("kooch_815_same_root");
    let mut state = ProjectState::new();
    state.active_project = Some(ActiveProject {
        manifest: crate::project::ProjectManifest::new("test"),
        root_path: project.clone(),
    });
    state.engine_root = Some(project.clone());
    let mut resources = Resources::new();
    resources.insert(state);
    let asset = written(&project.join("assets/Player.prefab"));

    delete_asset(&mut resources, &asset);

    assert!(!asset.exists(), "the project's own assets became read-only");
}
