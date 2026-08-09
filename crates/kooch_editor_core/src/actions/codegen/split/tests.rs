//! #558 — an existing project keeps working, and stops shipping the
//! editor.

use super::*;

/// A project in the shape the editor generated before #558.
fn old_project(dir: &Path, crate_name: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2024"

[workspace]

[lib]
crate-type = ["rlib", "dylib"]

[[bin]]
name = "{crate_name}"
path = "src/main.rs"

[dependencies]
kooch = {{ path = "/engine", features = ["editor", "physics", "remote", "dynamic"] }}
kooch_ecs = {{ path = "/engine/crates/kooch_ecs" }}
"#
        ),
    )
    .unwrap();
    std::fs::write(dir.join("src/main.rs"), old_scaffold(crate_name)).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "pub mod registrations;\nkooch::kooch_plugin_api::export_plugin!(ProjectPlugin);\n",
    )
    .unwrap();
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kooch_split_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn manifest(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("Cargo.toml")).unwrap()
}

/// 🔴 The line that made every release binary carry the editor.
#[test]
fn the_engine_dep_stops_asking_for_the_editor() {
    let dir = tmp("dep");
    old_project(&dir, "demo");

    split_authoring(&dir, "demo");

    let out = manifest(&dir);
    let line = kooch_line(&out).expect("the dep survived");
    assert!(
        !line.contains("features"),
        "the dependency still names features: {line}",
    );
    assert!(
        line.contains("path = \"/engine\""),
        "the path was lost: {line}"
    );
}

/// What replaces it: the project's own features, with the game as the
/// default and the editor opt-in.
#[test]
fn the_project_gains_its_own_features() {
    let dir = tmp("features");
    old_project(&dir, "demo");

    split_authoring(&dir, "demo");

    let out = manifest(&dir);
    assert!(out.contains("default = [\"game\"]"));
    assert!(out.contains("\"kooch/editor\""));
    assert!(
        out.find("[features]") < out.find("[dependencies]"),
        "the features block landed after the dependencies",
    );
}

/// The guarantee: a game build cannot produce the authoring binary,
/// because cargo refuses to build a target whose features are off.
#[test]
fn the_authoring_binary_is_gated() {
    let dir = tmp("gated");
    old_project(&dir, "demo");

    split_authoring(&dir, "demo");

    let out = manifest(&dir);
    let at = out.find("name = \"demo_editor\"").expect("declared");
    assert!(out[at..].contains("required-features = [\"editor\"]"));
    assert!(out[at..].contains("path = \"src/editor.rs\""));
    assert!(
        dir.join("src/editor.rs").is_file(),
        "no editor.rs was written"
    );
}

/// The old dispatcher went; what is left is only the game.
#[test]
fn main_becomes_the_game() {
    let dir = tmp("main");
    old_project(&dir, "demo");

    split_authoring(&dir, "demo");

    let main = std::fs::read_to_string(dir.join("src/main.rs")).unwrap();
    assert!(
        !main.contains("run_editor_with"),
        "main.rs still starts the editor",
    );
    assert!(
        !main.contains("--game"),
        "main.rs still matches on a mode flag"
    );
    assert!(
        main.contains("run_systems: true"),
        "main.rs runs no gameplay"
    );
}

/// 🔴 A migration that silently deleted someone's gameplay setup would
/// be worse than one that did nothing. An edited `main.rs` is left as it
/// is — the warning is what carries the news.
#[test]
fn an_edited_main_is_not_rewritten() {
    let dir = tmp("edited");
    old_project(&dir, "demo");
    let mine = old_scaffold("demo").replace(
        "app.run();",
        "app.add_plugin(my_game::Boot);\n        app.run();",
    );
    std::fs::write(dir.join("src/main.rs"), &mine).unwrap();

    split_authoring(&dir, "demo");

    assert_eq!(
        std::fs::read_to_string(dir.join("src/main.rs")).unwrap(),
        mine,
        "an edited main.rs was overwritten",
    );
    // The rest still migrated: the project builds, and its authoring
    // binary is the one the editor runs.
    assert!(manifest(&dir).contains("required-features = [\"editor\"]"));
}

/// A game links no plugin API, so an ungated export does not compile in
/// a game build.
#[test]
fn the_plugin_export_is_gated() {
    let dir = tmp("lib");
    old_project(&dir, "demo");

    split_authoring(&dir, "demo");

    let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    assert!(lib.contains("#[cfg(feature = \"editor\")]"));
    assert!(lib.contains("export_plugin!"), "the export was lost");
}

/// Opening a project twice must not append a second features block or a
/// third binary.
#[test]
fn migrating_twice_changes_nothing() {
    let dir = tmp("idempotent");
    old_project(&dir, "demo");

    split_authoring(&dir, "demo");
    let once = manifest(&dir);
    split_authoring(&dir, "demo");

    assert_eq!(manifest(&dir), once);
}

/// Same rule as the sibling migrations: a manifest the editor did not
/// write is somebody else's.
#[test]
fn a_foreign_manifest_is_left_alone() {
    let dir = tmp("foreign");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let original = "[package]\nname = \"mine\"\n\n[dependencies]\nserde = \"1\"\n";
    std::fs::write(dir.join("Cargo.toml"), original).unwrap();

    split_authoring(&dir, "mine");

    assert_eq!(manifest(&dir), original);
    assert!(!dir.join("src/editor.rs").exists());
}

/// A project created after #558 is already in this shape, and opening it
/// must not rewrite anything.
#[test]
fn a_fresh_project_is_untouched() {
    let dir = tmp("fresh");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        crate::project::generate_cargo_toml_for_test("demo", "/engine"),
    )
    .unwrap();
    std::fs::write(
        dir.join("src/main.rs"),
        crate::project::generate_main_rs("demo"),
    )
    .unwrap();
    let before = manifest(&dir);

    split_authoring(&dir, "demo");

    assert_eq!(manifest(&dir), before);
}
