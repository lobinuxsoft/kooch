use kooch_core::resource::Resources;

use super::{CarriedWorld, Phase, capture, hold, resume};

/// Serialises the tests that touch the holding directory.
static ALONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The guard, with a poisoned lock treated as held rather than fatal:
/// one failing test must not turn the rest into a second failure that
/// hides it.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    ALONE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 🔴 A fresh `SceneManager` already holds one untitled scene, and that is exactly the one worth
/// carrying: it has no file to be read back from, so a rebuild that dropped it would lose
/// everything in it with nothing on disk to recover from.
#[test]
fn an_untitled_scene_is_held() {
    let _alone = alone();
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::SceneManager::new());

    let held = hold(&mut resources);

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
    assert_eq!(hold(&mut resources), 0);
}

/// 🔴 From the #1158 smoke test (#1163): Rebuild & Run before the project connected held the editor's
/// empty untitled scene, and resuming it wiped the scene the new process had opened.
#[test]
fn a_disconnected_editor_holds_nothing() {
    let _alone = alone();
    let _ = std::fs::remove_dir_all(super::holding());
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::SceneManager::new());
    resources.insert(crate::remote_session::RemoteState::new());

    assert_eq!(capture(&mut resources), 0);
    assert!(
        resources.get::<CarriedWorld>().is_none(),
        "a carry was armed"
    );
    assert!(!super::holding().exists(), "a holding file was written");
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

/// 🔴 The claim the whole module rests on: what comes back is pointed at the file it belongs to, and
/// is still **unsaved**.
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
