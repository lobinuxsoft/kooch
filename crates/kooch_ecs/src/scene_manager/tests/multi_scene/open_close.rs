//! Opening, saving, closing and switching between several scenes at once.

use super::*;

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

/// Every entity has to know its home, or saving and closing cannot tell the two scenes apart.
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

/// The failure this prevents is duplication: saving one scene while another is open would write the
/// other's entities into both files, and the next load would spawn each of them twice.
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

/// Dirty is per scene. With two open, saving one must not claim the other's edits are safe.
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
