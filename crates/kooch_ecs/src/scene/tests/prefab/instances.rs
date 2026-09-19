//! as_instance_of and the editor's link: identities, references, membership, queries.

use super::*;

// -- as_instance_of -----------------------------------------------------

/// The whole reason ids are remapped: stamp the same prefab out twice without it and both copies
/// claim to be entity 1, so a reference to one resolves to whichever loaded last.
#[test]
fn two_instances_share_no_identity() {
    let prefab = document(vec![
        described("Root", vec![identified(1)]),
        described("Child", vec![parented_to(1), identified(2)]),
    ]);
    let into = Guid::new_v4();
    let mut allocator = PersistentIdAllocator::new();

    let a = prefab.as_instance_of(into, &mut allocator);
    let b = prefab.as_instance_of(into, &mut allocator);

    let ids = |doc: &SceneDocument| -> Vec<u64> {
        doc.entities
            .iter()
            .flat_map(|e| e.components.iter())
            .filter(|c| c.type_name.ends_with("PersistentId"))
            .filter_map(|c| match c.fields.first() {
                Some((_, ReflectValue::U64(raw))) => Some(*raw),
                _ => None,
            })
            .collect()
    };
    let (a_ids, b_ids) = (ids(&a), ids(&b));
    assert_eq!(a_ids.len(), 2);
    for id in &a_ids {
        assert!(!b_ids.contains(id), "id {id} appears in both instances");
    }
}

/// A remapped id is worth nothing if the references pointing at it are not remapped with it — the
/// child would end up parented to whatever else held its old id.
#[test]
fn a_reference_inside_an_instance_still_points_inside_it() {
    let prefab = document(vec![
        described("Root", vec![identified(7)]),
        described("Child", vec![parented_to(7)]),
    ]);
    let mut allocator = PersistentIdAllocator::new();
    let instance = prefab.as_instance_of(Guid::new_v4(), &mut allocator);

    let root_id = match &instance.entities[0].components[0].fields[0].1 {
        ReflectValue::U64(raw) => *raw,
        other => panic!("expected the root's id, got {other:?}"),
    };
    let parent_ref = match &instance.entities[1].components[0].fields[0].1 {
        ReflectValue::EntityRef(Some(EntityRef::Persistent { scene, id })) => (*scene, id.get()),
        other => panic!("expected a parent reference, got {other:?}"),
    };
    assert_ne!(root_id, 7, "the id was not remapped");
    assert_eq!(
        parent_ref,
        (None, root_id),
        "the child must follow the root to its new id, and stay scene-local",
    );
}

/// A reference naming another scene points outside the prefab. Remapping
/// it would repoint it at an unrelated entity of the containing scene.
#[test]
fn a_reference_into_another_scene_is_left_alone() {
    let elsewhere = Guid::new_v4();
    let prefab = document(vec![described(
        "Root",
        vec![ComponentDescription {
            type_name: "test::Follows".into(),
            fields: vec![(
                "target".into(),
                ReflectValue::EntityRef(Some(EntityRef::Persistent {
                    scene: Some(elsewhere),
                    id: EntityGuid::new(3).unwrap(),
                })),
            )],
        }],
    )]);
    let mut allocator = PersistentIdAllocator::new();
    let instance = prefab.as_instance_of(Guid::new_v4(), &mut allocator);

    assert_eq!(
        instance.entities[0].components[0].fields[0].1,
        ReflectValue::EntityRef(Some(EntityRef::Persistent {
            scene: Some(elsewhere),
            id: EntityGuid::new(3).unwrap(),
        })),
    );
}

/// The instance belongs to the scene that now contains it, which is what
/// makes saving that scene write the instance out.
#[test]
fn an_instance_becomes_a_member_of_the_scene_it_went_into() {
    let prefab = document(vec![described("Root", vec![identified(1)])]);
    let into = Guid::new_v4();
    let mut allocator = PersistentIdAllocator::new();
    assert_eq!(prefab.as_instance_of(into, &mut allocator).id, into);
}

// -- instantiate --------------------------------------------------------

#[test]
fn instantiating_hands_back_the_root() {
    let (mut resources, root, _) = world_with_a_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let into = Guid::new_v4();

    let spawned = crate::scene::sync::instantiate(&prefab, &mut resources, into).unwrap();

    // The entity handed back is the one with no parent — anything else and
    // the caller would be placing a child while its root stayed put.
    let registry = resources.get::<ComponentRegistry>().unwrap();
    let parent = registry.get_cpu::<Parent>().and_then(|s| s.get(spawned));
    assert!(
        parent.is_none() || parent.is_some_and(|p| p.entity == root),
        "the root of an instance must not be parented inside the prefab",
    );
    assert_ne!(spawned, root, "an instance is a copy, not the original");
}

#[test]
fn instantiating_a_multi_root_document_spawns_nothing() {
    let mut resources = setup_resources();
    let before = resources
        .get::<crate::archetype_registry::ArchetypeRegistry>()
        .map(|a| {
            a.iter_matching(&[])
                .map(|arch| arch.entities().len())
                .sum::<usize>()
        })
        .unwrap_or(0);

    let doc = document(vec![
        described("A", vec![identified(1)]),
        described("B", vec![identified(2)]),
    ]);
    assert!(crate::scene::sync::instantiate(&doc, &mut resources, Guid::new_v4()).is_err());

    let after = resources
        .get::<crate::archetype_registry::ArchetypeRegistry>()
        .map(|a| {
            a.iter_matching(&[])
                .map(|arch| arch.entities().len())
                .sum::<usize>()
        })
        .unwrap_or(0);
    assert_eq!(
        before, after,
        "a refused instance must not leave entities behind"
    );
}

// -- the editor's link ---------------------------------------------------

/// `spawn_prefab` is what a game calls, and a game wants entities rather than a relationship to
/// maintain. The link belongs to the editor's instancing, which attaches it afterwards.
#[test]
fn instancing_on_its_own_attaches_no_link() {
    let (mut resources, root) = world_with_a_deep_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let spawned = crate::scene::sync::instantiate(&prefab, &mut resources, Guid::new_v4()).unwrap();

    let linked = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<crate::prefab_instance::PrefabInstance>())
        .and_then(|s| s.get(spawned))
        .is_some();
    assert!(
        !linked,
        "a bare instantiation must not carry an editor link"
    );
}

/// And when the editor does attach it, the instance names the prefab it came from and starts with
/// nothing overridden — every field still follows the prefab.
#[test]
fn an_attached_link_names_its_prefab_and_overrides_nothing() {
    let (mut resources, root) = world_with_a_deep_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let source = Guid::new_v4();
    let (spawned, members) =
        crate::scene::sync::instantiate_members(&prefab, &mut resources, Guid::new_v4()).unwrap();

    crate::prefab_instance::attach(&mut resources, spawned, &members, source);

    let registry = resources.get::<ComponentRegistry>().unwrap();
    let link = registry
        .get_cpu::<crate::prefab_instance::PrefabInstance>()
        .and_then(|s| s.get(spawned))
        .expect("the editor's instancing links what it places");
    assert_eq!(link.source, Some(source));
    assert!(link.overrides().is_empty());
}

/// The link has to be visible to a query, or the propagation pass walks
/// past every instance it exists to find.
#[test]
fn a_linked_instance_is_findable_by_query() {
    let (mut resources, root) = world_with_a_deep_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let (spawned, members) =
        crate::scene::sync::instantiate_members(&prefab, &mut resources, Guid::new_v4()).unwrap();
    crate::prefab_instance::attach(&mut resources, spawned, &members, Guid::new_v4());

    let query = crate::query::Query::<&crate::prefab_instance::PrefabInstance>::new(&resources);
    let mut found = Vec::new();
    query.for_each_entity(|entity, _| found.push(entity));
    assert_eq!(found, vec![spawned], "the archetype never learned about it");
}

/// Every entity of an instance has to say which entity of the prefab it is, in both directions:
/// recording an override needs "this is entity 2", and propagation needs "entity 2 is this one". A
/// link on the root alone answers neither for a prefab with children.
#[test]
fn every_member_of_an_instance_knows_which_prefab_entity_it_is() {
    use crate::prefab_instance::{PrefabInstance, PrefabMember};

    let (mut resources, root) = world_with_a_deep_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let (spawned_root, members) =
        crate::scene::sync::instantiate_members(&prefab, &mut resources, Guid::new_v4()).unwrap();
    assert_eq!(members.len(), 3, "root, child and grandchild");

    crate::prefab_instance::attach(&mut resources, spawned_root, &members, Guid::new_v4());

    let registry = resources.get::<ComponentRegistry>().unwrap();
    let storage = registry.get_cpu::<PrefabMember>().unwrap();
    for (index, entity) in members.iter().enumerate() {
        let member = storage
            .get(*entity)
            .unwrap_or_else(|| panic!("member {index} was not tagged"));
        assert_eq!(
            member.index as usize, index,
            "member {index} points at the wrong prefab entity"
        );
        assert_eq!(
            member.root, spawned_root,
            "member {index} points at the wrong instance"
        );
    }
    // And the root is the one carrying the link itself.
    assert!(
        registry
            .get_cpu::<PrefabInstance>()
            .and_then(|s| s.get(spawned_root))
            .is_some()
    );
}

/// Two instances of one prefab must not claim each other's members, or
/// propagating to one would reach into the other.
#[test]
fn members_belong_to_the_instance_that_spawned_them() {
    use crate::prefab_instance::PrefabMember;

    let (mut resources, root) = world_with_a_deep_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let scene = Guid::new_v4();
    let source = Guid::new_v4();

    let (first, first_members) =
        crate::scene::sync::instantiate_members(&prefab, &mut resources, scene).unwrap();
    crate::prefab_instance::attach(&mut resources, first, &first_members, source);
    let (second, second_members) =
        crate::scene::sync::instantiate_members(&prefab, &mut resources, scene).unwrap();
    crate::prefab_instance::attach(&mut resources, second, &second_members, source);

    let registry = resources.get::<ComponentRegistry>().unwrap();
    let storage = registry.get_cpu::<PrefabMember>().unwrap();
    for entity in &first_members {
        assert_eq!(storage.get(*entity).unwrap().root, first);
    }
    for entity in &second_members {
        assert_eq!(storage.get(*entity).unwrap().root, second);
    }
}
