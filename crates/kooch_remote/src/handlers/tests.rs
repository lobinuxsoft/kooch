use super::*;
use kooch_ecs::hierarchy::{Children, Parent};

fn world() -> Resources {
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::allocator::EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(kooch_ecs::archetype_registry::ArchetypeRegistry::new());
    resources.insert(kooch_ecs::query::AccessTracker::new());
    resources.insert(Commands::new());
    resources
}

fn spawn(resources: &mut Resources) -> kooch_ecs::entity::Entity {
    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(resources).id();
    commands.apply(resources);
    resources.insert(commands);
    entity
}

fn attach(
    resources: &mut Resources,
    parent: kooch_ecs::entity::Entity,
    child: kooch_ecs::entity::Entity,
) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Parent>();
    registry.register_cpu_reflected::<Children>();
    if let Some(storage) = registry.get_cpu_mut::<Parent>() {
        storage.insert(child, Parent { entity: parent });
    }
    if let Some(storage) = registry.get_cpu_mut::<Children>() {
        let existing = storage.get(parent).map(|c| c.entities.clone());
        let mut entities = existing.unwrap_or_default();
        entities.push(child);
        storage.insert(parent, Children { entities });
    }
}

fn alive(resources: &Resources, entity: kooch_ecs::entity::Entity) -> bool {
    resources
        .get::<EntityAllocator>()
        .is_some_and(|a| a.is_alive(entity))
}

/// Despawning a parent has to take its whole subtree. A child left
/// behind holds a `Parent` pointing at a dead handle: nothing in the
/// hierarchy can reach it, its transform derives from an entity that
/// no longer exists, and it survives into the saved scene.
#[test]
fn despawning_a_parent_takes_its_descendants() {
    let mut resources = world();
    let root = spawn(&mut resources);
    let child = spawn(&mut resources);
    let grandchild = spawn(&mut resources);
    attach(&mut resources, root, child);
    attach(&mut resources, child, grandchild);

    despawn(&mut resources, EntityId::from(root)).unwrap();

    assert!(!alive(&resources, root));
    assert!(!alive(&resources, child), "the child outlived its parent");
    assert!(
        !alive(&resources, grandchild),
        "a deeper descendant outlived the subtree",
    );
}

/// A sibling is not a descendant. Over-collecting would silently
/// delete half the scene.
#[test]
fn despawning_leaves_everything_outside_the_subtree_alone() {
    let mut resources = world();
    let root = spawn(&mut resources);
    let child = spawn(&mut resources);
    let bystander = spawn(&mut resources);
    attach(&mut resources, root, child);

    despawn(&mut resources, EntityId::from(root)).unwrap();

    assert!(!alive(&resources, child));
    assert!(
        alive(&resources, bystander),
        "an unrelated entity was taken"
    );
}
