use std::path::Path;

use super::offers_main_scene;

#[test]
fn a_scene_in_the_project_offers_it() {
    assert!(offers_main_scene(
        Path::new("/p/assets/scenes/level.scene"),
        true
    ));
}

/// 🔴 A prefab is the same format, and a game opening one would start
/// with a single entity and no camera. The extension is the only thing
/// telling them apart.
#[test]
fn a_prefab_does_not() {
    assert!(!offers_main_scene(
        Path::new("/p/assets/props/rock.prefab"),
        true
    ));
}

#[test]
fn nothing_else_does_either() {
    for path in [
        "/p/assets/materials/red.kooch_material.ron",
        "/p/src/main.rs",
        "/p/Cargo.toml",
        "/p/assets/scenes",
    ] {
        assert!(!offers_main_scene(Path::new(path), true), "{path}");
    }
}

/// The engine's shipped assets are read-only here, and a path outside the
/// project cannot be stored in its manifest — the handler refuses it, and
/// offering an action that gets refused is worse than not offering it.
#[test]
fn a_read_only_root_offers_nothing() {
    assert!(!offers_main_scene(
        Path::new("/engine/assets/scenes/demo.scene"),
        false
    ));
}
