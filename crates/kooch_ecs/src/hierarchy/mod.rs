//! Scene hierarchy — Parent/Children components, GlobalTransform, and
//! systems for keeping them in sync.
//!
//! Provides a tree structure for entities. `Parent` marks an entity as a
//! child of another, while `Children` maintains the list of children.
//! `GlobalTransform` stores the world-space matrix computed from the
//! hierarchy chain.
//!
//! The hierarchy sync system runs in [`Stage::PostUpdate`] and keeps
//! `Parent` ↔ `Children` consistent, then propagates transforms top-down.

pub mod children;
pub mod descendants;
pub mod global_transform;
pub mod hierarchy_sync;
pub mod parent;
mod reparent;
pub mod transform_propagation;

pub use children::Children;
pub use descendants::collect_descendants;
pub use global_transform::GlobalTransform;
pub use hierarchy_sync::hierarchy_sync_system;
pub use parent::Parent;
pub use reparent::reparent;
pub use transform_propagation::transform_propagation_system;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocator::EntityAllocator;
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::commands::Commands;
    use crate::component::Component;
    use crate::component::ComponentRegistry;
    use crate::entity::Entity;
    use crate::query::AccessTracker;
    use crate::reflect::Reflect;
    use crate::transform::Transform;
    use glam::Vec3;
    use kooch_core::resource::Resources;

    fn setup() -> Resources {
        let mut resources = Resources::new();
        resources.insert(EntityAllocator::new());
        resources.insert(ComponentRegistry::new());
        resources.insert(ArchetypeRegistry::new());
        resources.insert(AccessTracker::new());
        resources.insert(Commands::new());

        // Register built-in components.
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.register_cpu_reflected::<Transform>();
            registry.register_cpu_reflected::<Parent>();
            registry.register_cpu_reflected::<Children>();
            registry.register_cpu_reflected::<GlobalTransform>();
        }
        resources
    }

    fn spawn_entity(resources: &mut Resources) -> Entity {
        let mut commands = resources.remove::<Commands>().unwrap();
        let entity = commands.spawn(resources).id();
        commands.apply(resources);
        resources.insert(commands);
        entity
    }

    fn insert_component<T: Component + Reflect>(
        resources: &mut Resources,
        entity: Entity,
        component: T,
    ) {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.register_cpu_reflected::<T>();
            registry
                .get_cpu_mut::<T>()
                .unwrap()
                .insert(entity, component);
        }
    }

    // -- Hierarchy sync tests --

    #[test]
    fn parent_populates_children() {
        let mut resources = setup();
        let parent = spawn_entity(&mut resources);
        let child = spawn_entity(&mut resources);

        insert_component(&mut resources, child, Parent { entity: parent });

        hierarchy_sync_system(&mut resources);

        let registry = resources.get::<ComponentRegistry>().unwrap();
        let children = registry.get_cpu::<Children>().unwrap().get(parent).unwrap();
        assert_eq!(children.entities, vec![child]);
    }

    #[test]
    fn removing_parent_clears_children() {
        let mut resources = setup();
        let parent = spawn_entity(&mut resources);
        let child = spawn_entity(&mut resources);

        insert_component(&mut resources, child, Parent { entity: parent });
        hierarchy_sync_system(&mut resources);

        // Remove Parent from child.
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.get_cpu_mut::<Parent>().unwrap().remove(child);
        }
        hierarchy_sync_system(&mut resources);

        let registry = resources.get::<ComponentRegistry>().unwrap();
        assert!(
            registry
                .get_cpu::<Children>()
                .unwrap()
                .get(parent)
                .is_none()
        );
    }

    #[test]
    fn reparent_updates_both_parents() {
        let mut resources = setup();
        let parent_a = spawn_entity(&mut resources);
        let parent_b = spawn_entity(&mut resources);
        let child = spawn_entity(&mut resources);

        insert_component(&mut resources, child, Parent { entity: parent_a });
        hierarchy_sync_system(&mut resources);

        // Reparent child to parent_b.
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry
                .get_cpu_mut::<Parent>()
                .unwrap()
                .get_mut(child)
                .unwrap()
                .entity = parent_b;
        }
        hierarchy_sync_system(&mut resources);

        let registry = resources.get::<ComponentRegistry>().unwrap();
        let children_storage = registry.get_cpu::<Children>().unwrap();
        assert!(children_storage.get(parent_a).is_none());
        assert_eq!(
            children_storage.get(parent_b).unwrap().entities,
            vec![child]
        );
    }

    #[test]
    fn entity_cannot_be_own_parent() {
        let mut resources = setup();
        let entity = spawn_entity(&mut resources);

        // Self-parenting — sync should handle gracefully (no infinite loop).
        insert_component(&mut resources, entity, Parent { entity });
        hierarchy_sync_system(&mut resources);

        // Should still produce a Children entry (the data is "correct" per the
        // Parent component, even if semantically weird). The important thing
        // is that no infinite loop occurs.
        let registry = resources.get::<ComponentRegistry>().unwrap();
        let children = registry.get_cpu::<Children>().unwrap().get(entity).unwrap();
        assert_eq!(children.entities, vec![entity]);
    }

    // -- Transform propagation tests --

    #[test]
    fn root_global_transform_equals_local() {
        let mut resources = setup();
        let entity = spawn_entity(&mut resources);

        insert_component(
            &mut resources,
            entity,
            Transform::from_position(Vec3::new(1.0, 2.0, 3.0)),
        );

        hierarchy_sync_system(&mut resources);
        transform_propagation_system(&mut resources);

        let registry = resources.get::<ComponentRegistry>().unwrap();
        let gt = registry
            .get_cpu::<GlobalTransform>()
            .unwrap()
            .get(entity)
            .unwrap();
        let expected = Transform::from_position(Vec3::new(1.0, 2.0, 3.0)).to_matrix();
        assert_eq!(gt.matrix, expected);
    }

    #[test]
    fn child_inherits_parent_transform() {
        let mut resources = setup();
        let parent = spawn_entity(&mut resources);
        let child = spawn_entity(&mut resources);

        insert_component(
            &mut resources,
            parent,
            Transform::from_position(Vec3::new(10.0, 0.0, 0.0)),
        );
        insert_component(
            &mut resources,
            child,
            Transform::from_position(Vec3::new(0.0, 5.0, 0.0)),
        );
        insert_component(&mut resources, child, Parent { entity: parent });

        hierarchy_sync_system(&mut resources);
        transform_propagation_system(&mut resources);

        let registry = resources.get::<ComponentRegistry>().unwrap();
        let gt = registry
            .get_cpu::<GlobalTransform>()
            .unwrap()
            .get(child)
            .unwrap();
        // Child world position = parent (10,0,0) + local (0,5,0) = (10,5,0).
        let col3 = gt.matrix.col(3);
        assert!((col3.x - 10.0).abs() < f32::EPSILON);
        assert!((col3.y - 5.0).abs() < f32::EPSILON);
        assert!((col3.z - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn nested_hierarchy_propagation() {
        let mut resources = setup();
        let grandparent = spawn_entity(&mut resources);
        let parent = spawn_entity(&mut resources);
        let child = spawn_entity(&mut resources);

        insert_component(
            &mut resources,
            grandparent,
            Transform::from_position(Vec3::new(1.0, 0.0, 0.0)),
        );
        insert_component(
            &mut resources,
            parent,
            Transform::from_position(Vec3::new(0.0, 2.0, 0.0)),
        );
        insert_component(
            &mut resources,
            child,
            Transform::from_position(Vec3::new(0.0, 0.0, 3.0)),
        );
        insert_component(
            &mut resources,
            parent,
            Parent {
                entity: grandparent,
            },
        );
        insert_component(&mut resources, child, Parent { entity: parent });

        hierarchy_sync_system(&mut resources);
        transform_propagation_system(&mut resources);

        let registry = resources.get::<ComponentRegistry>().unwrap();
        let gt = registry
            .get_cpu::<GlobalTransform>()
            .unwrap()
            .get(child)
            .unwrap();
        let col3 = gt.matrix.col(3);
        assert!((col3.x - 1.0).abs() < f32::EPSILON);
        assert!((col3.y - 2.0).abs() < f32::EPSILON);
        assert!((col3.z - 3.0).abs() < f32::EPSILON);
    }

    // -- Recursive despawn tests --

    #[test]
    fn collect_descendants_gathers_all() {
        let mut resources = setup();
        let root = spawn_entity(&mut resources);
        let child_a = spawn_entity(&mut resources);
        let child_b = spawn_entity(&mut resources);
        let grandchild = spawn_entity(&mut resources);

        insert_component(&mut resources, child_a, Parent { entity: root });
        insert_component(&mut resources, child_b, Parent { entity: root });
        insert_component(&mut resources, grandchild, Parent { entity: child_a });

        hierarchy_sync_system(&mut resources);

        let registry = resources.get::<ComponentRegistry>().unwrap();
        let descendants = collect_descendants(root, &registry);

        assert_eq!(descendants.len(), 4);
        assert!(descendants.contains(&root));
        assert!(descendants.contains(&child_a));
        assert!(descendants.contains(&child_b));
        assert!(descendants.contains(&grandchild));
    }

    #[test]
    fn collect_descendants_leaf_returns_only_self() {
        let mut resources = setup();
        let entity = spawn_entity(&mut resources);

        hierarchy_sync_system(&mut resources);

        let registry = resources.get::<ComponentRegistry>().unwrap();
        let descendants = collect_descendants(entity, &registry);
        assert_eq!(descendants, vec![entity]);
    }
}
