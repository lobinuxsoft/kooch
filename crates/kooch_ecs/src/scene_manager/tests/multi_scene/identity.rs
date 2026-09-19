//! A scene's identity across saves, copies and reused entity ids, and the epoch.

use super::*;

/// A scene keeps its identity across sessions, or every reference into it breaks on the next load.
#[test]
fn a_scene_keeps_its_identity_across_a_save_and_load() {
    let path = write_scene("multi_identity", &[1]);

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&path, &mut resources).unwrap();
    let first = manager.active_id().unwrap();

    manager.save(&mut resources).unwrap();

    let mut reloaded_resources = setup_resources();
    let mut reloaded = SceneManager::new();
    reloaded.load(&path, &mut reloaded_resources).unwrap();

    assert_eq!(
        reloaded.active_id(),
        Some(first),
        "the scene id must survive a round trip",
    );
}

/// A file written before scenes had identity gets one on load, and is marked dirty so it persists —
/// otherwise it would get a different id every session and references into it would never resolve.
#[test]
fn a_scene_file_without_an_id_is_marked_dirty_so_the_new_id_persists() {
    let path = tmp_path("multi_legacy");
    std::fs::write(&path, r#"(name: "Legacy", version: "0.1.0", entities: [])"#)
        .expect("writes a pre-identity file");

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&path, &mut resources).expect("still loads");

    assert!(
        manager.is_dirty(),
        "a scene that was just given an id has an unsaved change",
    );

    manager.save(&mut resources).expect("persists the id");
    let stored = std::fs::read_to_string(&path).unwrap();
    assert!(stored.contains("id:"), "the id reached the file");
    assert!(!manager.is_dirty());
}

/// Entity ids are scene-local, so two open scenes both having an entity 1 is ordinary. Resolving
/// references by id alone would collapse them and point every reference at whichever scene loaded
/// last — the same class of failure that made resolving parents by name unusable.
#[test]
fn two_scenes_may_reuse_the_same_entity_id_without_crossing_references() {
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::persistent_id::PersistentId;
    use crate::scene::SceneDocument;

    /// A component pointing at another entity, so the file carries a
    /// reference that the load pass has to resolve.
    #[derive(Debug, Default, Clone, PartialEq, kooch_ecs_macros::Reflect)]
    struct Link {
        target: Entity,
    }
    impl crate::component::Component for Link {}

    /// Builds a one-file scene whose single `Link` points at a sibling.
    fn write_linked_scene(name: &str, hp: u32) -> std::path::PathBuf {
        let mut resources = setup_resources();
        resources
            .get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<Link>();

        let (source, target) = {
            let mut commands = resources.remove::<Commands>().unwrap();
            let target = commands
                .spawn(&mut resources)
                .insert_reflected(super::single_scene::Health { hp })
                .id();
            let source = commands.spawn(&mut resources).id();
            commands.apply(&mut resources);
            resources.insert(commands);
            (source, target)
        };

        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<Link>()
        {
            storage.insert(source, Link { target });
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
            && let Some(current) = archetypes.entity_archetype(source)
        {
            let next =
                archetypes.archetype_after_add_dynamic(current, std::any::TypeId::of::<Link>());
            archetypes.register_entity(source, next);
        }

        let path = tmp_path(name);
        let mut manager = SceneManager::new();
        manager.save_as(path.clone(), &mut resources).unwrap();
        path
    }

    let first = write_linked_scene("multi_collide_a", 1);
    let second = write_linked_scene("multi_collide_b", 10);

    // Both files were written independently, so both allocated id 1.
    for path in [&first, &second] {
        let doc = SceneDocument::load(path).unwrap();
        let ids: Vec<u64> = doc
            .entities
            .iter()
            .flat_map(|e| &e.components)
            .filter(|c| c.type_name.ends_with("PersistentId"))
            .flat_map(|c| &c.fields)
            .filter_map(|(_, v)| match v {
                crate::reflect::ReflectValue::U64(raw) => Some(*raw),
                _ => None,
            })
            .collect();
        assert_eq!(ids, vec![1], "each scene numbers from 1 independently");
    }

    let mut resources = setup_resources();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Link>();

    let mut manager = SceneManager::new();
    manager.load(&first, &mut resources).unwrap();
    manager.open_additive(&second, &mut resources).unwrap();

    // Every link must point at a target in its own scene.
    let links: Vec<(Entity, Entity)> = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Link>())
        .map(|s| s.iter().map(|(&e, l)| (e, l.target)).collect())
        .unwrap_or_default();
    assert_eq!(links.len(), 2, "both scenes contributed a link");

    let owner = |entity: Entity| -> Guid {
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<SceneMember>())
            .and_then(|s| s.get(entity))
            .expect("every loaded entity has a home")
            .scene
    };

    for (source, target) in links {
        assert!(target.is_valid(), "the reference resolved");
        assert_eq!(
            owner(source),
            owner(target),
            "a reference crossed into the other scene — ids were resolved without their scene",
        );
        // And it really is the sibling that was pointed at, not itself.
        assert_ne!(source, target);
        assert_eq!(
            resources
                .get::<ComponentRegistry>()
                .and_then(|r| r.get_cpu::<PersistentId>())
                .and_then(|s| s.get(target))
                .map(|p| p.id.get()),
            Some(1),
            "the target is the entity that was numbered 1 in its own scene",
        );
    }
}

/// Entity names are free text, so a scene holding one called `grid:floor` contains the substring
/// `id:` without having an identity field.
#[test]
fn an_entity_name_containing_id_does_not_pass_for_a_scene_identity() {
    let path = tmp_path("multi_named_id");
    std::fs::write(
        &path,
        r#"(name: "Legacy", version: "0.1.0", entities: [(name: "grid:floor", components: [])])"#,
    )
    .expect("writes a pre-identity file whose text contains `id:`");

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&path, &mut resources).expect("loads");

    assert!(
        manager.is_dirty(),
        "the file has no identity field; the name merely looks like one",
    );
}

/// The same file open twice: identical entity ids, references that stay inside their own copy.
#[test]
fn two_copies_of_one_file_keep_their_ids_and_their_links() {
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::scene::SceneDocument;

    #[derive(Debug, Default, Clone, PartialEq, kooch_ecs_macros::Reflect)]
    struct Link {
        target: Entity,
    }
    impl crate::component::Component for Link {}

    let path = {
        let mut resources = setup_resources();
        resources
            .get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<Link>();
        let (source, target) = {
            let mut commands = resources.remove::<Commands>().unwrap();
            let target = commands
                .spawn(&mut resources)
                .insert_reflected(super::single_scene::Health { hp: 42 })
                .id();
            let source = commands.spawn(&mut resources).id();
            commands.apply(&mut resources);
            resources.insert(commands);
            (source, target)
        };
        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<Link>()
        {
            storage.insert(source, Link { target });
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
            && let Some(current) = archetypes.entity_archetype(source)
        {
            let next =
                archetypes.archetype_after_add_dynamic(current, std::any::TypeId::of::<Link>());
            archetypes.register_entity(source, next);
        }
        let path = tmp_path("multi_two_copies");
        let mut manager = SceneManager::new();
        manager.save_as(path.clone(), &mut resources).unwrap();
        path
    };

    // What the file says, so the assertion below is against the file
    // rather than against whatever the load happened to produce.
    let doc = SceneDocument::load(&path).unwrap();
    let file_ids: Vec<u64> = doc
        .entities
        .iter()
        .flat_map(|e| &e.components)
        .filter(|c| c.type_name.ends_with("PersistentId"))
        .flat_map(|c| &c.fields)
        .filter_map(|(_, v)| match v {
            crate::reflect::ReflectValue::U64(raw) => Some(*raw),
            _ => None,
        })
        .collect();
    assert!(!file_ids.is_empty(), "the fixture wrote no identities");

    let mut resources = setup_resources();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Link>();

    let mut manager = SceneManager::new();
    let first = {
        manager.load(&path, &mut resources).unwrap();
        manager.active_id().unwrap()
    };
    let second = manager.open_additive(&path, &mut resources).unwrap();
    assert_ne!(first, second);

    let owner = |entity: Entity| -> Guid {
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<SceneMember>())
            .and_then(|s| s.get(entity))
            .expect("every loaded entity has a home")
            .scene
    };

    // Every copy carries the file's ids, unchanged and therefore repeated.
    let live: Vec<(Guid, u64)> = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<crate::persistent_id::PersistentId>())
        .map(|s| s.iter().map(|(&e, p)| (owner(e), p.id.get())).collect())
        .unwrap_or_default();
    for scene in [first, second] {
        let mut ids: Vec<u64> = live
            .iter()
            .filter(|(s, _)| *s == scene)
            .map(|(_, id)| *id)
            .collect();
        ids.sort_unstable();
        let mut expected = file_ids.clone();
        expected.sort_unstable();
        assert_eq!(ids, expected, "copy {scene} did not keep the file's ids");
    }

    // And each copy's link stays inside it, even though both copies hold
    // an entity with the same id.
    let links: Vec<(Entity, Entity)> = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Link>())
        .map(|s| s.iter().map(|(&e, l)| (e, l.target)).collect())
        .unwrap_or_default();
    assert_eq!(links.len(), 2, "both copies contributed a link");
    for (source, target) in links {
        assert!(target.is_valid(), "the reference did not resolve");
        assert_eq!(
            owner(source),
            owner(target),
            "a link crossed into the other copy — ids were resolved without their instance",
        );
    }
}

/// 🔴 Every change to the set of loaded scenes moves the epoch.
#[test]
fn every_scene_change_moves_the_epoch() {
    let first = write_scene("epoch_a", &[1, 2]);
    let second = write_scene("epoch_b", &[3]);
    let mut resources = setup_resources();
    let mut manager = SceneManager::new();

    let mut seen = vec![manager.epoch()];
    let mut moved = |manager: &SceneManager, seen: &mut Vec<u32>, what: &str| {
        let now = manager.epoch();
        assert!(
            !seen.contains(&now),
            "{what} left the epoch at {now}, so a renderer cannot tell the world changed",
        );
        seen.push(now);
    };

    manager.load(&first, &mut resources).expect("loads");
    moved(&manager, &mut seen, "load");

    let added = manager
        .open_additive(&second, &mut resources)
        .expect("opens beside");
    moved(&manager, &mut seen, "open_additive");

    manager.new_scene();
    moved(&manager, &mut seen, "new_scene");

    manager.revert(added, &mut resources).expect("reverts");
    moved(&manager, &mut seen, "revert");

    assert!(manager.close(added, &mut resources), "closes");
    moved(&manager, &mut seen, "close");
}

/// ⚠️ And an ordinary edit does **not**.
#[test]
fn editing_a_scene_leaves_the_epoch_alone() {
    let path = write_scene("epoch_edit", &[7]);
    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&path, &mut resources).expect("loads");

    let after_load = manager.epoch();
    manager.mark_dirty();
    manager.mark_clean();

    assert_eq!(
        manager.epoch(),
        after_load,
        "editing a scene voided every shadow page in the pool",
    );
}

/// A scratch scene's contents are never written into somebody else's file.
#[test]
fn a_scratch_scene_is_never_folded_into_a_saved_one() {
    use crate::scene::SceneDocument;

    let first = write_scene("scratch_isolation", &[1, 2]);

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&first, &mut resources).unwrap();
    let first_id = manager.active_id().unwrap();

    // "Right-click the empty space and spawn something": a scene of its
    // own, never saved, holding one entity.
    let scratch = manager.new_scene();
    let stray = {
        let mut commands = resources.remove::<Commands>().unwrap();
        let entity = commands
            .spawn(&mut resources)
            .insert_reflected(super::single_scene::Health { hp: 999 })
            .id();
        commands.apply(&mut resources);
        resources.insert(commands);
        entity
    };
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<SceneMember>();
        if let Some(storage) = registry.get_cpu_mut::<SceneMember>() {
            storage.insert(stray, SceneMember::new(scratch));
        }
    }

    manager.set_active(first_id);
    manager
        .save_scene(first_id, &mut resources)
        .expect("saves the loaded scene");

    let written = SceneDocument::load(&first).expect("reads back");
    assert_eq!(
        written.entities.len(),
        2,
        "the scratch scene's entity was written into a file nobody aimed at",
    );
}
