use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::entity::Entity;
use kooch_ecs::name::Name;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::transform::Transform;

use super::*;

fn world() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    {
        let registry = r.get_mut::<ComponentRegistry>().unwrap();
        registry.register_cpu_reflected::<Name>();
        registry.register_cpu_reflected::<Transform>();
        registry.register_cpu_reflected::<kooch_ecs::hierarchy::Parent>();
    }
    r
}

fn spawn(resources: &mut Resources) -> Entity {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    entity
}

fn add<T: 'static>(resources: &mut Resources, entity: Entity) {
    let type_id = std::any::TypeId::of::<T>();
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    assert!(registry.insert_default_reflected(&type_id, entity));
}

fn set(
    resources: &mut Resources,
    entity: Entity,
    ty: std::any::TypeId,
    field: &str,
    v: ReflectValue,
) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.reflect_set_field(&ty, entity, field, v).unwrap();
}

/// The values come back, not just the component types.
#[test]
fn a_capture_carries_its_values() {
    let mut resources = world();
    let entity = spawn(&mut resources);
    add::<Transform>(&mut resources, entity);
    set(
        &mut resources,
        entity,
        std::any::TypeId::of::<Transform>(),
        "position",
        ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0)),
    );

    let state = capture(&resources, entity);
    let transform = state
        .components
        .iter()
        .find(|c| c.name.ends_with("Transform"))
        .expect("the transform was not captured");
    assert!(transform.fields.contains(&(
        "position".to_owned(),
        ReflectValue::Vec3(glam::Vec3::new(1.0, 2.0, 3.0)),
    )));
}

/// 🔴 The hierarchy link is never a captured component. It travels as its own field on both sides of
/// the wire, and a copy that carried its source's `Parent` as a value would point at whatever
/// entity handle happened to be at that index in the other process.
#[test]
fn the_parent_link_stays_home() {
    let mut resources = world();
    let entity = spawn(&mut resources);
    add::<Transform>(&mut resources, entity);
    add::<kooch_ecs::hierarchy::Parent>(&mut resources, entity);

    let state = capture(&resources, entity);
    assert!(
        !state.components.iter().any(|c| c.name.ends_with("Parent")),
        "the parent link was captured: {:?}",
        state.components.iter().map(|c| &c.name).collect::<Vec<_>>(),
    );
}

/// A component only the project knows is parked, not lost — it is
/// exactly the half a user wrote themselves.
#[test]
fn a_parked_component_is_captured() {
    let mut resources = world();
    let entity = spawn(&mut resources);
    resources.get_mut::<DynamicComponents>().unwrap().insert(
        entity,
        "the_game::Health",
        vec![("hp".to_owned(), ReflectValue::F32(7.0))],
    );

    let state = capture(&resources, entity);
    let health = state
        .components
        .iter()
        .find(|c| c.name == "the_game::Health")
        .expect("the parked component was not captured");
    assert_eq!(health.fields[0].1, ReflectValue::F32(7.0));
}

/// Captured here, restored there: the round trip is what paste and
/// undoing a despawn both rest on.
#[test]
fn a_restored_entity_matches() {
    let mut resources = world();
    let source = spawn(&mut resources);
    add::<Transform>(&mut resources, source);
    set(
        &mut resources,
        source,
        std::any::TypeId::of::<Transform>(),
        "scale",
        ReflectValue::Vec3(glam::Vec3::splat(4.0)),
    );
    let state = capture(&resources, source);

    let copy = spawn(&mut resources);
    restore_local(&mut resources, copy, &state);

    let scale = resources
        .get::<ComponentRegistry>()
        .unwrap()
        .reflect_get_fields(&std::any::TypeId::of::<Transform>(), copy)
        .expect("the copy has no transform")
        .into_iter()
        .find(|(name, _)| name == "scale")
        .map(|(_, value)| value);
    assert_eq!(scale, Some(ReflectValue::Vec3(glam::Vec3::splat(4.0))));
}

/// A copy says it is one, and an unnamed entity produces no name at all
/// rather than the string "Copy".
#[test]
fn a_copy_is_named_after_it() {
    let named = EntityState {
        name: Some("Hero".to_owned()),
        components: Vec::new(),
    };
    assert_eq!(copy_name(&named).as_deref(), Some("Hero Copy"));
    assert_eq!(copy_name(&EntityState::default()), None);
}

/// 🔴 A copy carries what the entity IS, not which file it came out of.
#[test]
fn a_copy_does_not_carry_its_scene() {
    let state = EntityState {
        name: Some("Hero".to_owned()),
        components: vec![
            ComponentState {
                name: std::any::type_name::<kooch_ecs::SceneMember>().to_owned(),
                fields: vec![(
                    "scene".to_owned(),
                    ReflectValue::String(kooch_core::Guid::new_v4().to_string()),
                )],
            },
            ComponentState {
                name: std::any::type_name::<Transform>().to_owned(),
                fields: Vec::new(),
            },
        ],
    };

    let copy = as_copy(&state);

    let names: Vec<&str> = copy.components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec![std::any::type_name::<Transform>()]);
}

/// 🔴 The copy of a prefab instance used to VANISH on save, and the answer then was to strip the
/// link — which left the author rebuilding the instance by hand (#1293). The link travels; what
/// made the copy vanish was that it kept the ORIGINAL's membership, so the save wrote it as neither
/// a whole entity nor a whole instance. `reroot_prefab` is what closes that.
#[test]
fn a_copy_keeps_the_prefab_and_its_own_membership() {
    use kooch_ecs::prefab_instance::{PrefabInstance, PrefabMember};

    let state = EntityState {
        name: Some("Player".to_owned()),
        components: vec![
            ComponentState {
                name: std::any::type_name::<PrefabMember>().to_owned(),
                fields: Vec::new(),
            },
            ComponentState {
                name: std::any::type_name::<PrefabInstance>().to_owned(),
                fields: Vec::new(),
            },
            ComponentState {
                name: std::any::type_name::<Transform>().to_owned(),
                fields: Vec::new(),
            },
        ],
    };

    let copy = as_copy(&state);

    let names: Vec<&str> = copy.components.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            std::any::type_name::<PrefabMember>(),
            std::any::type_name::<PrefabInstance>(),
            std::any::type_name::<Transform>(),
        ],
    );
}

fn component(name: &str) -> super::ComponentState {
    super::ComponentState {
        name: name.to_owned(),
        fields: Vec::new(),
    }
}

/// 🔴 #1287: a paste restored the original's identity onto a second entity, and every reference to
/// it then pointed at whichever the loader yielded.
#[test]
fn a_copy_leaves_the_identity_behind() {
    let identity = std::any::type_name::<kooch_ecs::PersistentId>();
    let state = EntityState {
        name: Some("Planet".into()),
        components: vec![super::ComponentState {
            name: identity.to_owned(),
            fields: vec![("id".to_owned(), ReflectValue::U64(1))],
        }],
    };
    for copy in [as_copy(&state), without_identity(&state)] {
        assert!(
            copy.components.iter().all(|c| c.name != identity),
            "a copy kept the original's id",
        );
    }
}

/// 🔴 #1293: a duplicate that stops following its prefab is an instance the author has to rebuild
/// by hand. The link travels; only the membership is re-pointed, by `reroot_prefab`.
#[test]
fn a_copy_keeps_the_prefab_it_came_from() {
    let instance = std::any::type_name::<kooch_ecs::prefab_instance::PrefabInstance>();
    let member = std::any::type_name::<kooch_ecs::prefab_instance::PrefabMember>();
    let root = EntityState {
        name: Some("Player".into()),
        components: vec![component(instance), component(member)],
    };
    let copy = as_copy(&root);
    assert!(
        copy.components.iter().any(|c| c.name == instance),
        "the link was dropped"
    );
    assert!(copy.components.iter().any(|c| c.name == member));

    // A member copied on its own belongs to no instance: kept, it would join the original's.
    let alone = EntityState {
        name: Some("Head".into()),
        components: vec![component(member)],
    };
    assert!(as_copy(&alone).components.iter().all(|c| c.name != member));
}

/// The copy's membership points at itself, so a prefab edit reaches both instances and an override
/// on one leaves the other alone.
#[test]
fn a_duplicate_belongs_to_itself() {
    use kooch_ecs::prefab_instance::{PrefabInstance, PrefabMember};

    let mut resources = world();
    let original = spawn(&mut resources);
    let copy = spawn(&mut resources);
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<PrefabInstance>();
        registry.register_cpu_reflected::<PrefabMember>();
        let source = kooch_core::Guid::new_v4();
        registry
            .get_cpu_mut::<PrefabInstance>()
            .unwrap()
            .insert(copy, PrefabInstance::new(source));
        registry.get_cpu_mut::<PrefabMember>().unwrap().insert(
            copy,
            PrefabMember {
                root: original,
                index: 0,
            },
        );
    }

    let copies = std::collections::HashMap::from([(original, copy)]);
    super::reroot_prefabs(&mut resources, &copies);

    let member = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<PrefabMember>())
        .and_then(|s| s.get(copy))
        .map(|member| member.root)
        .expect("the copy is still a member");
    assert_eq!(
        member, copy,
        "the copy still belongs to the original's instance"
    );
}

// ---- copying a subtree (#1292) ------------------------------------

/// A world with `Children` registered and a parent → child → grandchild chain.
fn family(resources: &mut Resources) -> (Entity, Entity, Entity) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<kooch_ecs::hierarchy::Children>();
    }
    let (root, child, grandchild) = (spawn(resources), spawn(resources), spawn(resources));
    for entity in [root, child, grandchild] {
        add::<Name>(resources, entity);
        add::<Transform>(resources, entity);
    }
    kooch_ecs::hierarchy::reparent(resources, child, Some(root));
    kooch_ecs::hierarchy::reparent(resources, grandchild, Some(child));
    (root, child, grandchild)
}

#[test]
fn a_capture_carries_the_whole_tree() {
    let mut resources = world();
    let (root, child, grandchild) = family(&mut resources);
    let tree = capture_tree(&resources, root);

    let sources: Vec<Entity> = tree.iter().map(|captured| captured.source).collect();
    assert_eq!(sources, vec![root, child, grandchild]);
    assert_eq!(tree[1].parent, Some(0), "the child hangs from the root");
    assert_eq!(tree[2].parent, Some(1), "and the grandchild from the child");
}

/// 🔴 #1292: duplicating an entity used to copy it alone, so a character arrived without its
/// camera and nothing said so.
#[test]
fn a_pasted_tree_has_the_same_shape() {
    let mut resources = world();
    let (root, _, _) = family(&mut resources);
    let tree = capture_tree(&resources, root);

    let copies = paste_tree_local(&mut resources, &tree, None);
    assert_eq!(copies.len(), 3, "every entity of the subtree was built");
    assert_eq!(parent_of(&resources, copies[1]), Some(copies[0]));
    assert_eq!(parent_of(&resources, copies[2]), Some(copies[1]));
    assert!(
        copies
            .iter()
            .all(|copy| !tree.iter().any(|c| c.source == *copy)),
        "the copies are new entities",
    );
}

/// A reference inside the subtree points at the copy; one out of it is left where it was.
#[test]
fn references_follow_the_copy() {
    use kooch_ecs::reflect::EntityRef;

    let mut resources = world();
    let (root, child, _) = family(&mut resources);
    let outside = spawn(&mut resources);
    let tree = capture_tree(&resources, root);
    let copies = std::collections::HashMap::from([(child, Entity::new(99, 0))]);

    let inward = EntityState {
        name: None,
        components: vec![ComponentState {
            name: "Link".to_owned(),
            fields: vec![
                (
                    "near".to_owned(),
                    ReflectValue::EntityRef(Some(EntityRef::live(child))),
                ),
                (
                    "far".to_owned(),
                    ReflectValue::EntityRef(Some(EntityRef::live(outside))),
                ),
            ],
        }],
    };
    let moved = remapped(&inward, &copies);
    let fields = &moved.components[0].fields;
    assert_eq!(
        fields[0].1,
        ReflectValue::EntityRef(Some(EntityRef::live(Entity::new(99, 0)))),
        "a reference into the subtree stayed on the original",
    );
    assert_eq!(
        fields[1].1,
        ReflectValue::EntityRef(Some(EntityRef::live(outside))),
        "a reference out of it was re-pointed",
    );
    assert_eq!(tree.len(), 3);
}

/// Copying a parent and its child selects one tree, not two: the child travels with the parent.
#[test]
fn a_selected_child_is_not_copied_twice() {
    let mut resources = world();
    let (root, child, grandchild) = family(&mut resources);
    assert_eq!(roots_of(&resources, &[root, child, grandchild]), vec![root]);
    assert_eq!(roots_of(&resources, &[child, grandchild]), vec![child]);
}

/// 🔴 A copied child came out scaled by 1/parent: `reparent` preserves the WORLD pose by rewriting
/// the local transform, so writing the captured one first and parenting after divided it.
#[test]
fn a_copied_child_keeps_its_transform() {
    let mut resources = world();
    let (root, child, _) = family(&mut resources);
    set(
        &mut resources,
        root,
        std::any::TypeId::of::<Transform>(),
        "scale",
        ReflectValue::Vec3(glam::Vec3::splat(14.0)),
    );
    set(
        &mut resources,
        child,
        std::any::TypeId::of::<Transform>(),
        "scale",
        ReflectValue::Vec3(glam::Vec3::splat(2.5)),
    );
    let tree = capture_tree(&resources, root);

    let copies = paste_tree_local(&mut resources, &tree, None);

    let scale_of = |resources: &Resources, entity: Entity| {
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.reflect_get_fields(&std::any::TypeId::of::<Transform>(), entity))
            .and_then(|fields| {
                fields
                    .into_iter()
                    .find_map(|(name, value)| match (name.as_str(), value) {
                        ("scale", ReflectValue::Vec3(scale)) => Some(scale),
                        _ => None,
                    })
            })
            .expect("the copy has a transform")
    };
    assert_eq!(scale_of(&resources, copies[0]), glam::Vec3::splat(14.0));
    assert_eq!(
        scale_of(&resources, copies[1]),
        glam::Vec3::splat(2.5),
        "the child was rescaled by its parent",
    );
}
