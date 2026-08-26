//! Several scenes open at once (#609).

use super::{setup_resources, tmp_path};
use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::entity::Entity;
use crate::scene_manager::SceneManager;
use crate::scene_member::SceneMember;
use kooch_core::Guid;
use kooch_core::resource::Resources;

/// Writes a scene file holding `count` entities with the given hp values.
fn write_scene(name: &str, hps: &[u32]) -> std::path::PathBuf {
    use crate::scene::SceneDocument;

    let mut resources = setup_resources();
    {
        let mut commands = resources.remove::<Commands>().unwrap();
        for &hp in hps {
            commands
                .spawn(&mut resources)
                .insert_reflected(super::single_scene::Health { hp });
        }
        commands.apply(&mut resources);
        resources.insert(commands);
    }

    let path = tmp_path(name);
    let mut manager = SceneManager::new();
    manager
        .save_as(path.clone(), &mut resources)
        .expect("writes the fixture");
    path
}

fn live_hps(resources: &Resources) -> Vec<u32> {
    use crate::query::Query;
    let mut hps: Vec<u32> = Query::<&super::single_scene::Health>::new(resources)
        .iter()
        .map(|h| h.hp)
        .collect();
    hps.sort_unstable();
    hps
}

fn members(resources: &Resources) -> Vec<(Entity, Guid)> {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<SceneMember>())
        .map(|s| s.iter().map(|(&e, m)| (e, m.scene)).collect())
        .unwrap_or_default()
}

#[test]
fn two_scenes_can_be_open_at_once() {
    let first = write_scene("multi_a", &[1, 2]);
    let second = write_scene("multi_b", &[10]);

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();

    manager
        .load(&first, &mut resources)
        .expect("loads the first");
    let second_id = manager
        .open_additive(&second, &mut resources)
        .expect("loads the second beside it");

    assert_eq!(manager.scenes().len(), 2);
    assert_eq!(live_hps(&resources), vec![1, 2, 10], "both scenes are live");
    assert_eq!(
        manager.active_id(),
        Some(second_id),
        "the scene just opened becomes active",
    );
}

/// Every entity has to know its home, or saving and closing cannot tell
/// the two scenes apart.
#[test]
fn each_entity_belongs_to_the_scene_that_loaded_it() {
    let first = write_scene("multi_owner_a", &[1]);
    let second = write_scene("multi_owner_b", &[10]);

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&first, &mut resources).unwrap();
    let first_id = manager.active_id().expect("a scene is active");
    let second_id = manager.open_additive(&second, &mut resources).unwrap();

    let owners = members(&resources);
    assert_eq!(owners.len(), 2, "both entities carry membership");
    assert!(owners.iter().any(|&(_, scene)| scene == first_id));
    assert!(owners.iter().any(|&(_, scene)| scene == second_id));
}

/// The failure this prevents is duplication: saving one scene while
/// another is open would write the other's entities into both files, and
/// the next load would spawn each of them twice.
#[test]
fn saving_one_scene_does_not_capture_the_other() {
    use crate::scene::SceneDocument;

    let first = write_scene("multi_save_a", &[1, 2]);
    let second = write_scene("multi_save_b", &[10]);

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&first, &mut resources).unwrap();
    let first_id = manager.active_id().unwrap();
    manager.open_additive(&second, &mut resources).unwrap();

    manager
        .save_scene(first_id, &mut resources)
        .expect("saves only the first");

    let written = SceneDocument::load(&first).expect("reads back");
    assert_eq!(
        written.entities.len(),
        2,
        "the first scene kept its own two entities and took none of the other's",
    );
}

/// A scene that is not the active one can be saved to a new path.
///
/// "Save As" on the scene the user right-clicked, which with several
/// open is not the one new entities land in. It must write that scene
/// and adopt the path for it alone — leaving the active scene's own
/// path, and the other's entities, untouched.
#[test]
fn a_scene_that_is_not_active_saves() {
    use crate::scene::SceneDocument;

    let first = write_scene("multi_saveas_a", &[1, 2]);
    let second = write_scene("multi_saveas_b", &[10]);

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&first, &mut resources).unwrap();
    let first_id = manager.active_id().unwrap();
    let second_id = manager.open_additive(&second, &mut resources).unwrap();
    assert_eq!(manager.active_id(), Some(second_id), "the second is active");

    let elsewhere = tmp_path("multi_saveas_out");
    manager
        .save_scene_as(first_id, elsewhere.clone(), &mut resources)
        .expect("saves the one that is not active");

    let written = SceneDocument::load(&elsewhere).expect("reads back");
    assert_eq!(
        written.entities.len(),
        2,
        "wrote the named scene, not the active one and not both",
    );
    assert_eq!(written.id, first_id, "and kept that scene's identity");
    assert_eq!(
        manager.scene(second_id).and_then(|s| s.path.clone()),
        Some(second),
        "the active scene's own path was left alone",
    );
    assert_eq!(
        manager.scene(first_id).and_then(|s| s.path.clone()),
        Some(elsewhere),
        "the saved scene adopted where it was written",
    );
}

/// Saving a scene that is not open is refused, rather than writing a
/// file nothing will ever load back into that identity.
#[test]
fn saving_a_scene_that_is_not_open_fails() {
    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    let stray = manager.save_scene_as(Guid::new_v4(), tmp_path("nope"), &mut resources);
    assert!(stray.is_err());
}

/// "Close the station" and "walk away from the station" are different
/// operations (#566); this is the first one.
#[test]
fn closing_a_scene_despawns_only_its_own_entities() {
    let first = write_scene("multi_close_a", &[1, 2]);
    let second = write_scene("multi_close_b", &[10]);

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&first, &mut resources).unwrap();
    let first_id = manager.active_id().unwrap();
    let second_id = manager.open_additive(&second, &mut resources).unwrap();

    assert!(manager.close(second_id, &mut resources));

    assert_eq!(
        live_hps(&resources),
        vec![1, 2],
        "only the second went away"
    );
    assert_eq!(manager.scenes().len(), 1);
    assert_eq!(
        manager.active_id(),
        Some(first_id),
        "closing the active scene falls back to one still open",
    );
}

#[test]
fn closing_a_scene_that_is_not_open_reports_it() {
    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    assert!(!manager.close(Guid::new_v4(), &mut resources));
}

/// The same file can be open twice, as two instances.
///
/// 🔴 This used to be refused, and the reason was real: a scene's
/// identity *was* its file's, so two copies claimed one id and every
/// `(scene, entity)` pair aliased.
///
/// Unity DOTS answers it a level up — instances of a subscene are "exact
/// copies of each other", told apart by the instance the load hands back
/// rather than by anything inside them. So the entities keep the ids the
/// file gives them, which is what makes a scene reload to exactly the
/// identities it was saved with, and the scene half of the pair says
/// which copy.
#[test]
fn the_same_scene_opens_twice_as_two_instances() {
    let path = write_scene("multi_twice", &[1, 2]);

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&path, &mut resources).unwrap();
    let first = manager.active_id().unwrap();

    let second = manager
        .open_additive(&path, &mut resources)
        .expect("a second copy is an instance, not a collision");

    assert_ne!(first, second, "both copies claimed one identity");
    assert_eq!(manager.scenes().len(), 2);
    assert_eq!(
        manager.instances_of(first).count(),
        2,
        "both copies name the same file",
    );
    assert_eq!(
        manager.scene(second).and_then(|s| s.source),
        Some(first),
        "the second copy forgot which file it came from",
    );

    // The entities are two sets of two, each belonging to its own copy.
    let members = members(&resources);
    assert_eq!(members.len(), 4, "one copy's entities went missing");
    let in_first = members.iter().filter(|(_, s)| *s == first).count();
    let in_second = members.iter().filter(|(_, s)| *s == second).count();
    assert_eq!((in_first, in_second), (2, 2));
}

/// Saving a second copy writes the **file's** identity, not the copy's.
///
/// Writing the instance id would rename the file every time a second copy
/// was saved, and break every reference that named it.
#[test]
fn saving_a_copy_keeps_the_files_identity() {
    use crate::scene::SceneDocument;

    let path = write_scene("multi_copy_save", &[7]);
    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&path, &mut resources).unwrap();
    let file_id = manager.active_id().unwrap();
    let copy = manager.open_additive(&path, &mut resources).unwrap();
    assert_ne!(copy, file_id);

    manager
        .save_scene(copy, &mut resources)
        .expect("the copy saves");

    let written = SceneDocument::load(&path).expect("reads back");
    assert_eq!(
        written.id, file_id,
        "saving a copy renamed the file to the copy's own id",
    );
}

/// Dirty is per scene. With two open, saving one must not claim the
/// other's edits are safe.
#[test]
fn dirty_state_is_tracked_per_scene() {
    let first = write_scene("multi_dirty_a", &[1]);
    let second = write_scene("multi_dirty_b", &[10]);

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&first, &mut resources).unwrap();
    let first_id = manager.active_id().unwrap();
    let second_id = manager.open_additive(&second, &mut resources).unwrap();

    manager.mark_dirty(); // marks the active one, which is the second
    assert!(manager.any_dirty());
    assert!(manager.scene(second_id).unwrap().dirty);
    assert!(
        !manager.scene(first_id).unwrap().dirty,
        "the other is clean"
    );

    manager.save_scene(second_id, &mut resources).unwrap();
    assert!(!manager.any_dirty(), "saving cleared the only dirty scene");
}

#[test]
fn the_active_scene_can_be_switched_but_only_to_an_open_one() {
    let first = write_scene("multi_active_a", &[1]);
    let second = write_scene("multi_active_b", &[10]);

    let mut resources = setup_resources();
    let mut manager = SceneManager::new();
    manager.load(&first, &mut resources).unwrap();
    let first_id = manager.active_id().unwrap();
    manager.open_additive(&second, &mut resources).unwrap();

    assert!(manager.set_active(first_id));
    assert_eq!(manager.active_id(), Some(first_id));

    assert!(
        !manager.set_active(Guid::new_v4()),
        "an unopened scene must not become active",
    );
    assert_eq!(
        manager.active_id(),
        Some(first_id),
        "a refused switch leaves the active scene alone",
    );
}

/// A scene keeps its identity across sessions, or every reference into it
/// breaks on the next load.
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

/// A file written before scenes had identity gets one on load, and is
/// marked dirty so it persists — otherwise it would get a different id
/// every session and references into it would never resolve.
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

/// Entity ids are scene-local, so two open scenes both having an entity 1
/// is ordinary. Resolving references by id alone would collapse them and
/// point every reference at whichever scene loaded last — the same class
/// of failure that made resolving parents by name unusable.
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

/// Entity names are free text, so a scene holding one called `grid:floor`
/// contains the substring `id:` without having an identity field. A
/// text search would call that file "already identified" and never
/// persist the id it was just given — a different scene id every session,
/// and no reference into it ever resolving.
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

/// The same file open twice: identical entity ids, references that stay
/// inside their own copy.
///
/// 🔴 This is the whole point of splitting a scene's identity from its
/// file's. The entities keep the ids the file gives them — which is what
/// makes a scene reload to exactly the identities it was saved with — and
/// the copies are told apart by the instance, the way Unity DOTS tells
/// subscene instances apart by the meta entity the load hands back.
///
/// Resolving by id alone would have every link in one copy pointing into
/// the other, because both copies really do contain entity 1.
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
