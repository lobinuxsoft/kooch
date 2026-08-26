//! Where a game looks for its scene when nobody told it.
//!
//! 🔴 Resolved against the working directory alone, a released game
//! opened by double-click starts **empty and silent**: the desktop leaves
//! the cwd at the user's home, the file is looked for there, and the
//! scene sitting beside the executable is never asked about.

use std::path::Path;

use super::scene_bootstrap::{Named, beside_exe, default_scene_path, scene_named_by};

use kooch_core::scene_paths::DEFAULT_SCENE_REL_PATH;

/// The candidate is under the executable's own **directory** — not the
/// executable's path with a name glued on, which is what
/// `exe.join(...)` would give and would name a folder that cannot exist.
#[test]
fn the_exe_candidate_sits_beside_it() {
    let candidate = beside_exe().expect("this test binary has a path");
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap();

    assert!(
        candidate.starts_with(dir),
        "{candidate:?} is not under {dir:?}"
    );
    assert!(candidate.ends_with(DEFAULT_SCENE_REL_PATH));
    assert!(
        !candidate.starts_with(&exe),
        "the scene was looked for inside the executable's own path",
    );
}

/// Nothing beside the test binary, so this falls through — which is the
/// case that keeps `cargo run` inside a project working, where the
/// executable lives in `target/debug/` and the scenes do not.
#[test]
fn without_one_beside_the_exe_the_cwd_answers() {
    let expected = std::env::current_dir()
        .unwrap()
        .join(DEFAULT_SCENE_REL_PATH);

    assert_eq!(default_scene_path(), expected);
}

/// And it is absolute either way: the loader joins a relative path onto
/// the cwd a second time, which would turn `scenes/x` under the exe into
/// `<cwd>/scenes/x` — the very path this exists to stop using.
#[test]
fn the_answer_is_always_absolute() {
    assert!(default_scene_path().is_absolute());
    assert!(beside_exe().is_some_and(|p| p.is_absolute()));
}

/// A packaged game: the scene is beside the binary and the cwd is
/// somewhere else entirely. This is the double-click case, and the one
/// that was broken.
#[test]
fn a_packaged_layout_is_found_from_anywhere() {
    let dist = std::env::temp_dir().join("kooch_boot_dist");
    let _ = std::fs::remove_dir_all(&dist);
    std::fs::create_dir_all(dist.join(kooch_core::scene_paths::SCENES_DIR)).unwrap();
    std::fs::write(dist.join(DEFAULT_SCENE_REL_PATH), "()").unwrap();

    // What `beside_exe` computes, with the exe's directory standing in
    // for `dist/` — the resolution rule, without moving this process.
    let candidate = dist.join(DEFAULT_SCENE_REL_PATH);
    assert!(
        candidate.exists(),
        "the layout a packaged game ships is not what is looked for",
    );
    assert_eq!(
        candidate.parent(),
        Some(dist.join(kooch_core::scene_paths::SCENES_DIR).as_path()),
    );
    assert!(!candidate.starts_with(std::env::current_dir().unwrap()));

    let _ = std::fs::remove_dir_all(&dist);
}

/// The relative path is shared with the editor, which writes the scene
/// there when it creates a project. Two constants would drift, and the
/// symptom is a game that ships a scene nothing looks for.
///
/// 🔴 Under `assets/` since #758: everything a game needs at runtime is
/// one tree, so packaging walks one place.
#[test]
fn the_layout_is_one_constant() {
    assert_eq!(
        Path::new(DEFAULT_SCENE_REL_PATH).parent().unwrap(),
        Path::new(kooch_core::scene_paths::SCENES_DIR),
    );
    assert!(DEFAULT_SCENE_REL_PATH.starts_with("assets/"));
}

/// 🔴 The case #808 exists for: a project whose starting scene is not
/// called `default.scene`.
///
/// Before this, `main_scene` was a field nothing read — the game opened
/// the convention path whatever the manifest said, so a project like this
/// shipped a build that started somewhere else, or empty, with no error.
#[test]
fn the_manifest_names_the_scene() {
    let dist = temp_dir("kooch_boot_named");
    std::fs::write(
        dist.join(kooch_core::scene_paths::PROJECT_MANIFEST_FILE),
        r#"(name: "g", main_scene: Some("assets/scenes/many_lights.scene"))"#,
    )
    .unwrap();

    assert_eq!(
        scene_named_by(&dist),
        Named::Scene(dist.join("assets/scenes/many_lights.scene")),
    );
    let _ = std::fs::remove_dir_all(&dist);
}

/// ⚠️ Both forms look plausible and only one resolves. Projects on disk
/// carry the short one — `roll-a-ball` did — and the failure was silent
/// on both sides: the editor's guard skipped the load, and the game never
/// looked at the field at all.
#[test]
fn the_short_form_still_resolves() {
    let dist = temp_dir("kooch_boot_short");
    std::fs::write(
        dist.join(kooch_core::scene_paths::PROJECT_MANIFEST_FILE),
        r#"(name: "g", main_scene: Some("scenes/default.scene"))"#,
    )
    .unwrap();

    assert_eq!(
        scene_named_by(&dist),
        Named::Scene(dist.join(DEFAULT_SCENE_REL_PATH)),
    );
    let _ = std::fs::remove_dir_all(&dist);
}

/// A directory with no manifest is not the same as one whose manifest is
/// silent: the first means "keep looking", the second means "the project
/// said nothing, use the convention". Collapsing them would make a game
/// read the manifest of whatever project the working directory happened
/// to be.
#[test]
fn silence_and_absence_are_different_answers() {
    let empty = temp_dir("kooch_boot_absent");
    assert_eq!(scene_named_by(&empty), Named::NoManifest);

    let quiet = temp_dir("kooch_boot_silent");
    std::fs::write(
        quiet.join(kooch_core::scene_paths::PROJECT_MANIFEST_FILE),
        r#"(name: "g", main_scene: None)"#,
    )
    .unwrap();
    assert_eq!(scene_named_by(&quiet), Named::Nothing);

    let _ = std::fs::remove_dir_all(&empty);
    let _ = std::fs::remove_dir_all(&quiet);
}

/// A fresh directory of its own per test: these run in parallel and a
/// shared one would have them writing each other's manifests.
fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
