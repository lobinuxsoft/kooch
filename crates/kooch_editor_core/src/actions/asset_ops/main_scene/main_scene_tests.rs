use std::path::{Path, PathBuf};

use super::relative_to_root;

/// 🔴 The one the whole feature rests on. An absolute path in the
/// manifest works on the machine that clicked and nowhere else — and the
/// failure arrives as a game that opens an empty scene, on somebody
/// else's computer, with nothing in the log pointing here.
#[test]
fn a_scene_is_stored_relative_to_the_project() {
    let root = Path::new("/home/someone/projects/game");
    let scene = root.join("assets/scenes/many_lights.scene");

    assert_eq!(
        relative_to_root(root, &scene).as_deref(),
        Some("assets/scenes/many_lights.scene"),
    );
}

/// The stored form keeps `assets/`, because that is what both readers
/// join against the project root. The short form resolves to nothing and
/// both of them fail quietly — see `normalise_main_scene`.
#[test]
fn the_assets_prefix_is_kept() {
    let root = Path::new("/p");
    let stored = relative_to_root(root, &root.join("assets/scenes/a.scene")).unwrap();
    assert!(stored.starts_with("assets/"), "{stored}");
}

/// A scene somewhere else on disk is not this project's starting scene.
/// Storing `../other/x.scene` would be a manifest that only resolves
/// from one directory on one machine.
#[test]
fn a_scene_outside_the_project_is_refused() {
    let root = Path::new("/home/someone/projects/game");
    let outside = PathBuf::from("/home/someone/projects/other/assets/scenes/a.scene");

    assert_eq!(relative_to_root(root, &outside), None);
}

#[test]
fn the_root_itself_is_not_a_scene() {
    let root = Path::new("/p");
    assert_eq!(relative_to_root(root, root), None);
}
