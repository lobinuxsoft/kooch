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

/// The pieces `hold` needs to find an entity and write it out.
fn world() -> Resources {
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::allocator::EntityAllocator::new());
    resources.insert(kooch_ecs::component::ComponentRegistry::new());
    resources.insert(kooch_ecs::archetype_registry::ArchetypeRegistry::new());
    resources.insert(kooch_ecs::query::AccessTracker::new());
    resources.insert(kooch_ecs::commands::Commands::new());
    resources.insert(kooch_ecs::dynamic_components::DynamicComponents::new());
    resources
}

/// Spawns one entity authored in `scene`, archetype included.
fn spawn_in(resources: &mut Resources, scene: kooch_core::Guid) {
    use kooch_ecs::SceneMember;
    use kooch_ecs::archetype_registry::ArchetypeRegistry;
    use kooch_ecs::component::ComponentRegistry;

    let mut commands = resources.remove::<kooch_ecs::commands::Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);

    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<SceneMember>();
        if let Some(storage) = registry.get_cpu_mut::<SceneMember>() {
            storage.insert(entity, SceneMember::new(scene));
        }
    }
    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
        && let Some(current) = archetypes.entity_archetype(entity)
    {
        let next =
            archetypes.archetype_after_add_dynamic(current, std::any::TypeId::of::<SceneMember>());
        archetypes.register_entity(entity, next);
    }
}

/// 🔴 The #1159 smoke test, and the second time #1163 has bitten: a fresh `SceneManager` holds an
/// untitled scene that stays empty while a connected project's world is the one on screen, because
/// the World panel reads the project's scenes over the wire and not this list. Held and resumed,
/// that empty scene replaced the world the rebuilt project had just opened.
#[test]
fn an_empty_scene_is_not_held() {
    let _alone = alone();
    let _ = std::fs::remove_dir_all(super::holding());
    let mut resources = world();
    resources.insert(kooch_ecs::SceneManager::new());

    assert_eq!(hold(&mut resources), 0, "an empty scene was held");
    assert!(
        resources.get::<CarriedWorld>().is_none(),
        "a carry was armed"
    );
}

/// What the carry is for: a scene with work in it and no file to read it back from.
#[test]
fn a_populated_scene_is_held() {
    let _alone = alone();
    let mut resources = world();
    let manager = kooch_ecs::SceneManager::new();
    let scene = manager.scenes()[0].id;
    resources.insert(manager);
    spawn_in(&mut resources, scene);

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
