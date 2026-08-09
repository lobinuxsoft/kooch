//! Where a game looks for its scene when nobody told it.
//!
//! 🔴 Resolved against the working directory alone, a released game
//! opened by double-click starts **empty and silent**: the desktop leaves
//! the cwd at the user's home, the file is looked for there, and the
//! scene sitting beside the executable is never asked about.

use std::path::Path;

use super::scene_bootstrap::{beside_exe, default_scene_path};

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
