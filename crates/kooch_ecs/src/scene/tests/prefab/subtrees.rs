//! What a prefab takes from the world: the subtree, its root, its name and identity.

use super::*;

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
    // Attach the subtree under something outside it first, so there is a parent that *could* leak.
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

/// Three levels, so the test covers a link that is neither to the root nor
/// from it. Two levels can pass while a deeper chain is silently flattened.
pub(super) fn world_with_a_deep_subtree() -> (kooch_core::resource::Resources, crate::entity::Entity)
{
    let mut resources = setup_resources();
    let (root, child, grandchild) = {
        let mut commands = resources.remove::<Commands>().unwrap();
        let root = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 9, max_hp: 9 })
            .id();
        let child = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 2, max_hp: 2 })
            .id();
        let grandchild = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 1, max_hp: 1 })
            .id();
        commands.apply(&mut resources);
        resources.insert(commands);
        (root, child, grandchild)
    };
    named(&mut resources, root, "Turret");
    named(&mut resources, child, "Barrel");
    named(&mut resources, grandchild, "Muzzle");
    parent_child(&mut resources, root, child);
    parent_child(&mut resources, child, grandchild);
    (resources, root)
}

/// The name of an entity, for identifying spawned copies.
fn name_of(resources: &kooch_core::resource::Resources, entity: crate::entity::Entity) -> String {
    resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<crate::name::Name>())
        .and_then(|storage| storage.get(entity))
        .map(|name| name.value.clone())
        .unwrap_or_default()
}

fn parent_of(
    resources: &kooch_core::resource::Resources,
    entity: crate::entity::Entity,
) -> Option<crate::entity::Entity> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<Parent>()?
        .get(entity)
        .map(|parent| parent.entity)
}

#[test]
fn a_subtree_takes_every_level_not_just_the_first() {
    let (mut resources, root) = world_with_a_deep_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let mut names: Vec<&str> = prefab.entities.iter().map(|e| e.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Barrel", "Muzzle", "Turret"]);
}

/// The test the whole prefab feature rests on and that nothing covered: a captured hierarchy has to
/// come back as a hierarchy.
#[test]
fn instancing_rebuilds_the_whole_hierarchy() {
    let (mut resources, root) = world_with_a_deep_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let spawned_root = crate::scene::sync::instantiate(&prefab, &mut resources, Guid::new_v4())
        .expect("a captured subtree has exactly one root");

    // The copies are found through the links themselves rather than by name — names are not unique,
    // and resolving by name is the bug this is checking has not come back.
    assert_eq!(name_of(&resources, spawned_root), "Turret");
    assert_ne!(spawned_root, root, "an instance is a copy");
    assert_eq!(
        parent_of(&resources, spawned_root),
        None,
        "the instance root must be free of the parent it was authored under",
    );

    let children: Vec<crate::entity::Entity> = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<Children>())
        .and_then(|storage| storage.get(spawned_root))
        .map(|children| children.entities.clone())
        .unwrap_or_default();
    // `Children` is derived by the hierarchy sync system, which does not run here, so the link is
    // read from the owning side: find the entity whose `Parent` is the spawned root.
    let _ = children;

    let spawned_child = find_child_of(&resources, spawned_root).expect("Barrel lost its parent");
    assert_eq!(name_of(&resources, spawned_child), "Barrel");

    let spawned_grandchild =
        find_child_of(&resources, spawned_child).expect("Muzzle lost its parent");
    assert_eq!(name_of(&resources, spawned_grandchild), "Muzzle");

    // And the copy is wired to the copy, not back to the original.
    assert_ne!(spawned_child, spawned_grandchild);
    assert_eq!(
        parent_of(&resources, spawned_grandchild),
        Some(spawned_child)
    );
}

/// The one entity whose `Parent` is `parent`, searched over the world.
fn find_child_of(
    resources: &kooch_core::resource::Resources,
    parent: crate::entity::Entity,
) -> Option<crate::entity::Entity> {
    let registry = resources.get::<ComponentRegistry>()?;
    let storage = registry.get_cpu::<Parent>()?;
    let allocator = resources.get::<crate::allocator::EntityAllocator>()?;
    (0..64u32)
        .filter_map(|index| {
            let entity = crate::entity::Entity::new(index, 0);
            allocator.is_alive(entity).then_some(entity)
        })
        .find(|entity| storage.get(*entity).is_some_and(|p| p.entity == parent))
}

/// Two instances of one prefab must not end up sharing a child, which is
/// what an un-remapped reference looks like from the outside.
#[test]
fn two_instances_do_not_share_children() {
    let (mut resources, root) = world_with_a_deep_subtree();
    let prefab = SceneDocument::from_ecs_subtree(&mut resources, root);
    let into = Guid::new_v4();

    let first = crate::scene::sync::instantiate(&prefab, &mut resources, into).unwrap();
    let second = crate::scene::sync::instantiate(&prefab, &mut resources, into).unwrap();

    assert_ne!(first, second);
    let first_child = find_child_of(&resources, first).expect("first instance lost its child");
    let second_child = find_child_of(&resources, second).expect("second instance lost its child");
    assert_ne!(
        first_child, second_child,
        "both instances claimed the same child — identity was not remapped",
    );
}
