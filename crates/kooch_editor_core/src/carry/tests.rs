use kooch_core::resource::Resources;

use super::{CarriedWorld, Phase, capture, is_held, resume};

/// Serialises the tests that touch the holding directory.
///
/// 🔴 `holding()` is ONE path per machine, on purpose: the world it
/// holds has to survive the process that wrote it. So `capture` clears
/// it and `resume` deletes it, and two tests doing that at once is one
/// wiping the other's files between its write and its assert. Nothing
/// here is worth a per-test directory — that would stop testing the
/// path the editor actually uses.
static ALONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The guard, with a poisoned lock treated as held rather than fatal:
/// one failing test must not turn the rest into a second failure that
/// hides it.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    ALONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 🔴 A fresh `SceneManager` already holds one untitled scene, and that
/// is exactly the one worth carrying: it has no file to be read back
/// from, so a rebuild that dropped it would lose everything in it with
/// nothing on disk to recover from.
#[test]
fn an_untitled_scene_is_held() {
    let _alone = alone();
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::SceneManager::new());

    let held = capture(&mut resources);

    assert_eq!(held, 1, "the untitled scene was left behind");
    let carried = resources.get::<CarriedWorld>().expect("a carry");
    assert!(
        carried.scenes[0].origin.is_none(),
        "an untitled scene must come back untitled, not adopted under its holding file",
    );
    assert!(carried.scenes[0].held.exists(), "nothing was written out");
}

/// No scene manager at all — the launch screen, before a project is
/// open. Rebuild is not reachable there, but capture must not panic
/// reaching for one.
#[test]
fn no_manager_holds_nothing() {
    let mut resources = Resources::new();
    assert_eq!(capture(&mut resources), 0);
}

/// 🔴 Loading into a project that is still compiling goes nowhere, and
/// the carry would be spent against a process that never saw it.
#[test]
fn a_disconnected_project_waits() {
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::SceneManager::new());
    resources.insert(CarriedWorld {
        scenes: Vec::new(),
        phase: Phase::Waiting,
    });

    let actions = resume(&mut resources);

    assert!(
        actions.is_empty(),
        "the world was sent to a project that is not there"
    );
    assert!(
        resources.get::<CarriedWorld>().is_some(),
        "the carry was dropped while the project was still starting",
    );
}

/// The holding files are the editor's own, not the project's. Anything
/// that treats a path as a project asset has to be able to tell.
#[test]
fn a_held_file_is_recognised() {
    let dir = std::env::temp_dir().join("kooch_carried_world");
    assert!(is_held(&dir.join("abc.scene")));
    assert!(!is_held(std::path::Path::new(
        "/proj/assets/scenes/level.scene"
    )));
}

/// 🔴 The claim the whole module rests on: what comes back is pointed at
/// the file it belongs to, and is still **unsaved**.
///
/// Adopted under its holding file, a later save would write the author's
/// scene into a temporary directory and delete it. Marked clean, the
/// editor would claim the work was written out by the one action that
/// did not write it.
#[test]
fn a_restored_scene_keeps_its_file_and_stays_dirty() {
    let _alone = alone();
    let mut manager = kooch_ecs::SceneManager::new();
    let id = manager.active_id().expect("a scene");
    let mut resources = Resources::new();
    resources.insert(manager);

    let origin = std::path::PathBuf::from("/proj/assets/scenes/level.scene");
    resources.insert(CarriedWorld {
        scenes: vec![super::Held {
            id,
            origin: Some(origin.clone()),
            held: std::env::temp_dir().join("kooch_carried_world/x.scene"),
        }],
        phase: Phase::Sent,
    });

    let actions = resume(&mut resources);

    assert!(actions.is_empty(), "the loads were queued a second time");
    let manager = resources.get::<kooch_ecs::SceneManager>().expect("manager");
    assert_eq!(
        manager.scene(id).and_then(|s| s.path.clone()),
        Some(origin),
        "the scene stayed adopted under its holding file",
    );
    assert!(
        manager.any_dirty(),
        "the editor claimed the carried edits were saved",
    );
    assert!(
        resources.get::<CarriedWorld>().is_none(),
        "the carry outlived the restore and would fire again",
    );
}
