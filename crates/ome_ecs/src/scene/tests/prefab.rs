//! Capturing one entity as a prefab, and stamping it back out (#611).
//!
//! The document half of these tests needs no world: remapping identity is
//! a transform from one `SceneDocument` to another, and testing it that way
//! keeps what is being asserted about identity separate from whether the
//! ECS happened to spawn things in a given order.

use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::hierarchy::{Children, Parent};
use crate::persistent_id::{EntityGuid, PersistentIdAllocator};
use crate::reflect::{EntityRef, ReflectValue};
use crate::scene::{ComponentDescription, EntityDescription, SceneDocument, SceneError};
use ome_core::Guid;

use super::{Health, setup_resources};

/// `Parent` and `Children` are both written by hand: a test that relied on
/// the hierarchy sync running would be asserting two things at once.
fn parent_child(
    resources: &mut ome_core::resource::Resources,
    parent: crate::entity::Entity,
    child: crate::entity::Entity,
) {
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        registry.register_cpu_reflected::<Parent>();
        registry.register_cpu_reflected::<Children>();
        if let Some(storage) = registry.get_cpu_mut::<Parent>() {
            storage.insert(child, Parent { entity: parent });
        }
        if let Some(storage) = registry.get_cpu_mut::<Children>() {
            storage.insert(
                parent,
                Children {
                    entities: vec![child],
                },
            );
        }
    }
    super::add_to_archetype(resources, child, std::any::TypeId::of::<Parent>());
    super::add_to_archetype(resources, parent, std::any::TypeId::of::<Children>());
}

fn named(resources: &mut ome_core::resource::Resources, entity: crate::entity::Entity, name: &str) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<crate::name::Name>();
    registry.insert_default_reflected(&std::any::TypeId::of::<crate::name::Name>(), entity);
    if let Some(storage) = registry.get_cpu_mut::<crate::name::Name>()
        && let Some(value) = storage.get_mut(entity)
    {
        value.value = name.to_owned();
    }
    super::add_to_archetype(
        resources,
        entity,
        std::any::TypeId::of::<crate::name::Name>(),
    );
}

/// A root with one child, plus an unrelated entity that must not be
/// dragged in.
fn world_with_a_subtree() -> (
    ome_core::resource::Resources,
    crate::entity::Entity,
    crate::entity::Entity,
) {
    let mut resources = setup_resources();
    let (root, child, outsider) = {
        let mut commands = resources.remove::<Commands>().unwrap();
        let root = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 9, max_hp: 9 })
            .id();
        let child = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 1, max_hp: 1 })
            .id();
        let outsider = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 5, max_hp: 5 })
            .id();
        commands.apply(&mut resources);
        resources.insert(commands);
        (root, child, outsider)
    };
    named(&mut resources, root, "Ball");
    named(&mut resources, child, "Trail");
    named(&mut resources, outsider, "Ground");
    parent_child(&mut resources, root, child);
    (resources, root, outsider)
}

#[test]
fn a_subtree_takes_the_root_and_its_descendants_and_nothing_else() {
    let (mut resources, root, _) = world_with_a_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);

    let names: Vec<&str> = prefab.entities.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names.len(), 2, "root and child only, got {names:?}");
    assert!(names.contains(&"Ball"));
    assert!(names.contains(&"Trail"));
    assert!(
        !names.contains(&"Ground"),
        "an unrelated entity was dragged into the prefab",
    );
}

/// The root's `Parent` points at whatever it was attached to while being
/// authored, which is not part of the prefab. Writing it would leave the
/// file referring to an entity the file does not contain.
#[test]
fn the_prefab_root_carries_no_parent() {
    let (mut resources, root, outsider) = world_with_a_subtree();
    // Attach the subtree under something outside it first, so there is a
    // parent that *could* leak.
    parent_child(&mut resources, outsider, root);

    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let root_desc = prefab.entities.iter().find(|e| e.name == "Ball").unwrap();
    assert!(
        !root_desc
            .components
            .iter()
            .any(|c| c.type_name.ends_with("Parent")),
        "the prefab root kept a parent from the scene it was captured in",
    );
    // The child's link is inside the file, so it stays.
    let child_desc = prefab.entities.iter().find(|e| e.name == "Trail").unwrap();
    assert!(
        child_desc
            .components
            .iter()
            .any(|c| c.type_name.ends_with("Parent")),
    );
}

#[test]
fn a_prefab_is_named_after_the_entity_it_came_from() {
    let (mut resources, root, _) = world_with_a_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    assert_eq!(prefab.name, "Ball");
}

#[test]
fn a_prefab_takes_its_own_identity_not_the_scene_it_came_from() {
    let (mut resources, root, _) = world_with_a_subtree();
    let a = SceneDocument::from_ecs_subtree(&mut resources, root);
    let b = SceneDocument::from_ecs_subtree(&mut resources, root);
    assert_ne!(a.id, b.id, "two captures are two scenes");
}

// -- root_index ---------------------------------------------------------

fn described(name: &str, components: Vec<ComponentDescription>) -> EntityDescription {
    EntityDescription {
        name: name.into(),
        parent_index: None,
        parent: None,
        components,
    }
}

fn parented_to(id: u64) -> ComponentDescription {
    ComponentDescription {
        type_name: "ome_ecs::hierarchy::Parent".into(),
        fields: vec![(
            "entity".into(),
            ReflectValue::EntityRef(Some(EntityRef::Persistent {
                scene: None,
                id: EntityGuid::new(id).unwrap(),
            })),
        )],
    }
}

fn identified(id: u64) -> ComponentDescription {
    ComponentDescription {
        type_name: "ome_ecs::persistent_id::PersistentId".into(),
        fields: vec![("id".into(), ReflectValue::U64(id))],
    }
}

fn document(entities: Vec<EntityDescription>) -> SceneDocument {
    SceneDocument {
        id: Guid::new_v4(),
        name: "Prefab".into(),
        version: "0.1.0".into(),
        entities,
    }
}

#[test]
fn one_tree_has_one_root() {
    let doc = document(vec![
        described("Root", vec![identified(1)]),
        described("Child", vec![parented_to(1)]),
    ]);
    assert_eq!(doc.root_index().unwrap(), 0);
}

/// Instancing as a unit needs one entity to place and parent. Several
/// roots have no such entity, and silently picking the first would attach
/// the rest to the scene root where nothing would ever move them.
#[test]
fn several_roots_cannot_be_instanced_as_a_unit() {
    let doc = document(vec![
        described("A", vec![identified(1)]),
        described("B", vec![identified(2)]),
        described("C", vec![parented_to(1)]),
    ]);
    assert!(matches!(
        doc.root_index(),
        Err(SceneError::NotASingleRoot { roots: 2 })
    ));
}

#[test]
fn an_empty_document_has_no_root() {
    assert!(matches!(
        document(vec![]).root_index(),
        Err(SceneError::NotASingleRoot { roots: 0 })
    ));
}

// -- as_instance_of -----------------------------------------------------

/// The whole reason ids are remapped: stamp the same prefab out twice
/// without it and both copies claim to be entity 1, so a reference to one
/// resolves to whichever loaded last.
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

/// A remapped id is worth nothing if the references pointing at it are not
/// remapped with it — the child would end up parented to whatever else
/// held its old id.
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
