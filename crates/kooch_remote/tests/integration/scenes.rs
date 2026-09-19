//! Scenes over the wire: listing, loading, saving, dirt, spawning into one, reverting.

use super::*;

/// The project's open scenes travel with the per-frame snapshot. 🔴 The World panel's roots come
/// from here; the editor's own manager lists a scene nothing belongs to.
#[test]
fn the_open_scene_set_is_listed() {
    let mut resources = ecs();
    let mut manager = kooch_ecs::SceneManager::new();
    manager.set_current(std::path::PathBuf::from("assets/scenes/many_lights.scene"));
    let id = manager
        .active_id()
        .expect("new manager has an active scene");
    resources.insert(manager);

    let scenes = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { scenes, .. } => scenes.expect("the host has a SceneManager"),
        other => panic!("list: {other:?}"),
    };

    assert_eq!(scenes.len(), 1);
    assert_eq!(scenes[0].id, id, "the project's id, not one minted here");
    assert_eq!(
        scenes[0].path.as_deref(),
        Some("assets/scenes/many_lights.scene"),
    );
    assert!(scenes[0].active);
}

/// A host with no `SceneManager` says nothing rather than no scenes, or the World panel blanks
/// every frame.
#[test]
fn a_host_without_scenes_says_nothing() {
    let mut resources = ecs();
    match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { scenes, .. } => assert_eq!(scenes, None),
        other => panic!("list: {other:?}"),
    }
}

/// Loading a *second* scene teaches the project's manager. 🔴 Loads twice, because the boot scene
/// hides the bug on one.
#[test]
fn loading_a_second_scene_teaches_the_manager() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());

    let dir = std::env::temp_dir().join("kooch_remote_load_scene");
    std::fs::create_dir_all(&dir).expect("temp dir");

    let mut write_scene = |name: &str, id: &str| {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(r#"(id: "{id}", name: "{name}", version: "0.1.0", entities: [])"#),
        )
        .expect("write scene");
        path
    };
    let first = write_scene("station.scene", "ae0b881d-c3e2-49e1-ae19-cf8c3db5288e");
    let second = write_scene("hangar.scene", "019023f7-29d5-433e-98c8-e79461209106");

    let mut load = |resources: &mut Resources, path: &std::path::Path| {
        call(
            resources,
            Method::LoadScene {
                path: path.to_string_lossy().into_owned(),
            },
        );
    };
    load(&mut resources, &first);
    load(&mut resources, &second);

    let scenes = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { scenes, .. } => scenes.expect("the host has a SceneManager"),
        other => panic!("list: {other:?}"),
    };
    assert_eq!(scenes.len(), 1, "the second load replaced the first");
    assert_eq!(
        scenes[0].id,
        "019023f7-29d5-433e-98c8-e79461209106"
            .parse::<kooch_core::Guid>()
            .expect("a well-formed id"),
        "the project still named the scene it had left",
    );
    assert_eq!(
        scenes[0].path.as_deref(),
        Some(second.to_string_lossy().as_ref()),
    );
    assert!(scenes[0].active);

    let _ = std::fs::remove_file(&first);
    let _ = std::fs::remove_file(&second);
}

/// Saving over the wire writes one scene and keeps its id. 🔴 Writing the world doubled entities on
/// the next load and re-minted the id each save.
#[test]
fn saving_writes_one_scene_and_keeps_its_id() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());

    let dir = std::env::temp_dir().join("kooch_remote_save_scene");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let source = dir.join("station.scene");
    let id = "ae0b881d-c3e2-49e1-ae19-cf8c3db5288e";
    std::fs::write(
        &source,
        format!(r#"(id: "{id}", name: "Station", version: "0.1.0", entities: [])"#),
    )
    .expect("write scene");

    call(
        &mut resources,
        Method::LoadScene {
            path: source.to_string_lossy().into_owned(),
        },
    );

    let out = dir.join("written.scene");
    call(
        &mut resources,
        Method::SaveScene {
            path: out.to_string_lossy().into_owned(),
            scene: None,
        },
    );

    let written = kooch_ecs::scene::SceneDocument::load(&out).expect("reads back");
    assert_eq!(
        written.id,
        id.parse::<kooch_core::Guid>().expect("a well-formed id"),
        "the save minted a new identity for a scene that already had one",
    );

    let _ = std::fs::remove_file(&source);
    let _ = std::fs::remove_file(&out);
}

/// A host with no `SceneManager` refuses to save rather than writing the whole world into the file.
#[test]
fn saving_without_a_manager_is_refused() {
    let mut resources = ecs();
    let out = std::env::temp_dir().join("kooch_remote_no_manager.scene");
    // Cleared first: the assertion below is "nothing was written", and a
    // leftover from an earlier run would fail a correct implementation.
    let _ = std::fs::remove_file(&out);
    let response = handle(
        &Request {
            id: 1,
            notify: false,
            method: Method::SaveScene {
                path: out.to_string_lossy().into_owned(),
                scene: None,
            },
        },
        &mut resources,
    );
    assert!(
        matches!(response.payload, ResponsePayload::Error(_)),
        "wrote something without knowing which scene it was",
    );
    assert!(!out.exists(), "a refused save left a file behind");
}

/// An edit marks the scene it changed, and a save clears it. 🔴 Nothing marked scenes dirty before,
/// so the asterisk was inert.
#[test]
fn an_edit_marks_the_scene_dirty() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());

    let dir = std::env::temp_dir().join("kooch_remote_dirty");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("station.scene");
    std::fs::write(
        &path,
        r#"(id: "ae0b881d-c3e2-49e1-ae19-cf8c3db5288e", name: "Station", version: "0.1.0", entities: [])"#,
    )
    .expect("write scene");

    let dirty = |resources: &mut Resources| -> bool {
        match call(resources, Method::ListEntities { since: None }) {
            ResponseData::Entities { scenes, .. } => scenes.expect("open set")[0].dirty,
            other => panic!("list: {other:?}"),
        }
    };

    call(
        &mut resources,
        Method::LoadScene {
            path: path.to_string_lossy().into_owned(),
        },
    );
    assert!(!dirty(&mut resources), "a freshly loaded scene is clean");

    call(
        &mut resources,
        Method::Spawn {
            name: None,
            scene: None,
            parent: None,
        },
    );
    assert!(
        dirty(&mut resources),
        "spawning left the scene reading clean"
    );

    let out = dir.join("written.scene");
    call(
        &mut resources,
        Method::SaveScene {
            path: out.to_string_lossy().into_owned(),
            scene: None,
        },
    );
    assert!(!dirty(&mut resources), "the save did not clear the flag");

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&out);
}

/// The scene that changed is marked, not the active one — with two open they differ.
#[test]
fn the_edited_scene_is_the_one_marked() {
    use kooch_ecs::SceneManager;

    let mut resources = ecs();
    let mut manager = SceneManager::new();
    let active = manager.active_id().expect("a scene");
    // A second scene, open but not active. Registered by hand: opening one additively is not a
    // remote method, and what is under test is which of the two an edit marks.
    let elsewhere = kooch_core::Guid::new_v4();
    assert!(
        !manager.mark_scene_dirty(elsewhere),
        "a scene that is not open cannot be marked",
    );
    resources.insert(manager);

    // An entity belonging to no scene falls back to the active one.
    let entity = match call(
        &mut resources,
        Method::Spawn {
            name: None,
            scene: None,
            parent: None,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("spawn: {other:?}"),
    };
    let manager = resources.get::<SceneManager>().expect("manager");
    assert!(
        manager.scene(active).expect("open").dirty,
        "an unowned entity marks the scene that will adopt it",
    );
    let _ = entity;
}

/// A spawn lands where it was asked for, not in the active scene's root.
#[test]
fn a_spawn_lands_in_the_scene_it_names() {
    use kooch_ecs::SceneManager;

    let mut resources = ecs();
    let mut manager = SceneManager::new();
    let active = manager.active_id().expect("a scene");
    let elsewhere = manager.new_scene();
    assert!(manager.set_active(active), "put the active one back");
    resources.insert(manager);

    let spawned = |resources: &mut Resources, scene, parent| match call(
        resources,
        Method::Spawn {
            name: None,
            scene,
            parent,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("spawn: {other:?}"),
    };
    let home = |resources: &Resources, entity: kooch_remote::protocol::EntityId| {
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<kooch_ecs::SceneMember>())
            .and_then(|s| s.get(kooch_ecs::entity::Entity::from(entity)))
            .map(|m| m.scene)
    };

    let plain = spawned(&mut resources, None, None);
    assert_eq!(home(&resources, plain), Some(active), "unnamed went astray");

    let named = spawned(&mut resources, Some(elsewhere), None);
    assert_eq!(
        home(&resources, named),
        Some(elsewhere),
        "the scene it named was ignored for the active one",
    );

    // A parent already names the scene, so the child follows it even
    // though the request says nothing about scenes.
    let child = spawned(&mut resources, None, Some(named));
    assert_eq!(
        home(&resources, child),
        Some(elsewhere),
        "a child was authored into a scene its parent is not in",
    );

    // And a parent wins over a scene that disagrees: an entity's scene IS its parent's, so
    // honouring both would write the child to a file its parent is not in.
    let contested = spawned(&mut resources, Some(active), Some(named));
    assert_eq!(
        home(&resources, contested),
        Some(elsewhere),
        "the scene field overrode the parent, splitting a tree across two files",
    );
}

/// A new scene opens beside the others and takes the spawn that asked for it — the World panel's
/// empty-space gesture.
#[test]
fn a_new_scene_opens_unsaved() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());

    let opened = match call(&mut resources, Method::NewScene) {
        ResponseData::SceneOpened { scene } => scene,
        other => panic!("new_scene: {other:?}"),
    };

    let scenes = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { scenes, .. } => scenes.expect("open set"),
        other => panic!("list: {other:?}"),
    };
    assert_eq!(scenes.len(), 2, "it replaced the scene already open");
    let fresh = scenes.iter().find(|s| s.id == opened).expect("listed");
    assert_eq!(fresh.path, None, "an unsaved scene claimed a file");
    assert!(fresh.active, "new entities would not land in it");
    assert!(!fresh.dirty, "an empty scene has nothing to lose yet");

    let entity = match call(
        &mut resources,
        Method::Spawn {
            name: Some("First".into()),
            scene: Some(opened),
            parent: None,
        },
    ) {
        ResponseData::Spawned { entity } => entity,
        other => panic!("spawn: {other:?}"),
    };
    let home = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_ecs::SceneMember>())
        .and_then(|s| s.get(kooch_ecs::entity::Entity::from(entity)))
        .map(|m| m.scene);
    assert_eq!(home, Some(opened), "the entity did not join the new scene");
}

/// Reverting throws away one scene's edits and leaves the rest alone.
#[test]
fn a_revert_reads_the_file_back() {
    use kooch_ecs::SceneManager;

    let mut resources = ecs();
    resources.insert(SceneManager::new());

    let dir = std::env::temp_dir().join("kooch_remote_revert");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("station.scene");
    std::fs::write(
        &path,
        r#"(id: "ae0b881d-c3e2-49e1-ae19-cf8c3db5288e", name: "Station", version: "0.1.0", entities: [])"#,
    )
    .expect("write scene");

    call(
        &mut resources,
        Method::LoadScene {
            path: path.to_string_lossy().into_owned(),
        },
    );
    call(
        &mut resources,
        Method::Spawn {
            name: Some("Mistake".into()),
            scene: None,
            parent: None,
        },
    );

    let named = |resources: &mut Resources| -> Vec<String> {
        match call(resources, Method::ListEntities { since: None }) {
            ResponseData::Entities { entities, .. } => {
                entities.iter().filter_map(|e| e.name.clone()).collect()
            }
            other => panic!("list: {other:?}"),
        }
    };
    assert!(named(&mut resources).contains(&"Mistake".to_owned()));

    call(&mut resources, Method::RevertScene { scene: None });
    assert!(
        !named(&mut resources).contains(&"Mistake".to_owned()),
        "the edit survived a discard",
    );

    let scenes = match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { scenes, .. } => scenes.expect("open set"),
        other => panic!("list: {other:?}"),
    };
    assert!(!scenes[0].dirty, "a reverted scene still read as edited");

    let _ = std::fs::remove_file(&path);
}

/// A never-saved scene refuses to revert: there is nothing to read back, and despawning would
/// delete work.
#[test]
fn an_unsaved_scene_refuses_to_revert() {
    let mut resources = ecs();
    resources.insert(kooch_ecs::SceneManager::new());
    call(
        &mut resources,
        Method::Spawn {
            name: Some("Work".into()),
            scene: None,
            parent: None,
        },
    );

    let response = handle(
        &Request {
            id: 1,
            notify: false,
            method: Method::RevertScene { scene: None },
        },
        &mut resources,
    );
    assert!(
        matches!(response.payload, ResponsePayload::Error(_)),
        "an unsaved scene reverted to nothing",
    );

    match call(&mut resources, Method::ListEntities { since: None }) {
        ResponseData::Entities { entities, .. } => assert_eq!(
            entities.len(),
            1,
            "the refusal despawned the work it could not restore",
        ),
        other => panic!("list: {other:?}"),
    }
}
