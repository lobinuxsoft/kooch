//! #1187 — which engine features a project's scenes need, read against the real engine manifest.

use std::path::PathBuf;

use super::*;

/// roll-a-ball's `game` on 2026-09-16, the build that shipped without its blocks.
const PROJECT: &str = r#"
[package]
name = "demo"

[features]
default = ["game"]
game = ["kooch/physics", "kooch/gravity", "kooch/character", "kooch/camera", "kooch/audio"]
editor = ["game", "kooch/editor", "kooch/blockmesh"]

[dependencies]
kooch = { path = "../kooch" }
"#;

fn scene(components: &[&str]) -> Vec<(PathBuf, String)> {
    let text = components
        .iter()
        .map(|c| format!("                    type_name: \"{c}\",\n"))
        .collect();
    vec![(PathBuf::from("assets/scenes/level.scene"), text)]
}

#[test]
fn a_block_scene_needs_blockmesh() {
    let scenes = scene(&[
        "kooch_ecs::transform::Transform",
        "kooch_blockmesh::block::Block",
    ]);

    let missing = missing_in(PROJECT, ENGINE_MANIFEST, &[], &scenes);

    assert_eq!(missing.len(), 1, "{missing:?}");
    assert_eq!(missing[0].feature, "blockmesh");
    assert_eq!(missing[0].component, "kooch_blockmesh::block::Block");
}

/// `character` turns on `gravity`, which turns on `physics`: a scene of bodies is not short of
/// anything, and the editor-only feature does not count for a game build.
#[test]
fn implied_and_default_crates_count() {
    let scenes = scene(&[
        "kooch_physics::components::body::physics_body::PhysicsBody",
        "kooch_render::material::asset::Material",
        "kooch_input::actions::action::Action",
        "roll_a_ball::Coin",
    ]);

    assert!(missing_in(PROJECT, ENGINE_MANIFEST, &[], &scenes).is_empty());
}

#[test]
fn a_selected_feature_brings_it_in() {
    let scenes = scene(&["kooch_blockmesh::block::Block"]);

    let with_editor = missing_in(PROJECT, ENGINE_MANIFEST, &["editor".to_owned()], &scenes);

    assert!(with_editor.is_empty(), "{with_editor:?}");
}

#[test]
fn the_game_feature_gains_it() {
    let scenes = scene(&["kooch_blockmesh::block::Block"]);
    let missing = missing_in(PROJECT, ENGINE_MANIFEST, &[], &scenes);

    let fixed = with_features(PROJECT, &missing).unwrap();

    assert!(fixed.contains(
        r#"game = ["kooch/physics", "kooch/gravity", "kooch/character", "kooch/camera", "kooch/audio", "kooch/blockmesh"]"#
    ));
    assert!(missing_in(&fixed, ENGINE_MANIFEST, &[], &scenes).is_empty());
    assert_eq!(
        with_features(&fixed, &missing).unwrap(),
        fixed,
        "added twice"
    );
}

/// On disk, twice: the second pass finds nothing to add and leaves the file as the first wrote it.
#[test]
fn ensure_fixes_a_project_once() {
    let root = std::env::temp_dir().join(format!("kooch_1187_{}", std::process::id()));
    let scenes = root.join("assets/scenes");
    std::fs::create_dir_all(&scenes).unwrap();
    std::fs::write(root.join("Cargo.toml"), PROJECT).unwrap();
    std::fs::write(
        scenes.join("level.scene"),
        "type_name: \"kooch_blockmesh::block::Block\",\n",
    )
    .unwrap();

    let unfixed = ensure(&root, &[]);
    let once = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let again = ensure(&root, &[]);
    let twice = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let _ = std::fs::remove_dir_all(&root);

    assert!(unfixed.is_empty() && again.is_empty());
    let game = once.lines().find(|l| l.starts_with("game = [")).unwrap();
    assert!(game.contains("\"kooch/blockmesh\""), "{game}");
    assert_eq!(once, twice);
}
