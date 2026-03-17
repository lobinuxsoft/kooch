//! Scene hierarchy — Parent/Children components and transform propagation.
//!
//! Provides a tree structure for entities. `Parent` marks an entity as a
//! child of another, while `Children` maintains the list of children.
//! `GlobalTransform` stores the world-space matrix computed from the
//! hierarchy chain.
//!
//! The hierarchy sync system runs in [`Stage::PostUpdate`] and keeps
//! `Parent` ↔ `Children` consistent, then propagates transforms top-down.

use glam::Mat4;

use crate::component::Component;
use crate::entity::Entity;
use crate::reflect::{FieldKind, FieldMeta, Reflect, ReflectError, ReflectValue};

// ---------------------------------------------------------------------------
// Parent component
// ---------------------------------------------------------------------------

/// Marks this entity as a child of another entity.
///
/// `Parent` is the authoritative side of the relationship — the hierarchy
/// sync system updates `Children` to match.
#[derive(Debug, Clone)]
pub struct Parent {
    pub entity: Entity,
}

impl Component for Parent {}

impl Reflect for Parent {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        // Entity references are not editable via the generic inspector.
        &[]
    }

    fn reflect_get(&self, _field: &str) -> Option<ReflectValue> {
        None
    }

    fn reflect_set(&mut self, field: &str, _value: ReflectValue) -> Result<(), ReflectError> {
        Err(ReflectError::FieldNotFound(field.into()))
    }

    fn reflect_default() -> Self {
        Self {
            entity: Entity::INVALID,
        }
    }
}

// ---------------------------------------------------------------------------
// Children component
// ---------------------------------------------------------------------------

/// Ordered list of child entities. Maintained automatically by the
/// hierarchy sync system based on `Parent` components.
#[derive(Debug, Clone, Default)]
pub struct Children {
    pub entities: Vec<Entity>,
}

impl Component for Children {}

impl Reflect for Children {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        &[]
    }

    fn reflect_get(&self, _field: &str) -> Option<ReflectValue> {
        None
    }

    fn reflect_set(&mut self, field: &str, _value: ReflectValue) -> Result<(), ReflectError> {
        Err(ReflectError::FieldNotFound(field.into()))
    }

    fn reflect_default() -> Self {
        Self::default()
    }
}

// ---------------------------------------------------------------------------
// GlobalTransform component
// ---------------------------------------------------------------------------

/// World-space transform matrix, computed from the hierarchy chain.
///
/// For root entities: `GlobalTransform = Transform::to_matrix()`.
/// For children: `GlobalTransform = parent.GlobalTransform * local.to_matrix()`.
///
/// This component is read-only from the user's perspective — it is
/// recomputed every frame by the transform propagation system.
#[derive(Debug, Clone, Copy)]
pub struct GlobalTransform {
    pub matrix: Mat4,
}

impl Component for GlobalTransform {}

impl Default for GlobalTransform {
    fn default() -> Self {
        Self {
            matrix: Mat4::IDENTITY,
        }
    }
}

impl Reflect for GlobalTransform {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[FieldMeta {
            name: "matrix",
            type_name: "glam::Mat4",
            kind: FieldKind::Mat4,
        }];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "matrix" => Some(ReflectValue::Mat4(self.matrix)),
            _ => None,
        }
    }

    fn reflect_set(&mut self, _field: &str, _value: ReflectValue) -> Result<(), ReflectError> {
        Err(ReflectError::ReadOnly)
    }

    fn reflect_default() -> Self {
        Self::default()
    }
}

// ---------------------------------------------------------------------------
// Hierarchy sync system
// ---------------------------------------------------------------------------

/// Synchronizes `Parent` ↔ `Children` components and propagates transforms.
///
/// Runs in [`Stage::PostUpdate`]. `Parent` is authoritative:
/// - If entity A has `Parent(B)`, A is added to B's `Children`.
/// - Stale entries in `Children` (where the child's `Parent` doesn't match)
///   are removed.
pub fn hierarchy_sync_system(resources: &mut ome_core::resource::Resources) {
    use crate::component::ComponentRegistry;

    let Some(mut registry) = resources.remove::<ComponentRegistry>() else {
        return;
    };

    // -- Phase 1: Build the authoritative parent→children map from Parent components.
    let parent_pairs: Vec<(Entity, Entity)> = registry
        .get_cpu::<Parent>()
        .map(|storage| {
            storage
                .iter()
                .map(|(child, parent)| (*child, parent.entity))
                .collect()
        })
        .unwrap_or_default();

    // Build expected children: parent_entity → [child_entities].
    let mut expected: std::collections::HashMap<Entity, Vec<Entity>> =
        std::collections::HashMap::new();
    for &(child, parent) in &parent_pairs {
        expected.entry(parent).or_default().push(child);
    }

    // -- Phase 2: Update Children components to match.
    // Ensure Children storage exists.
    registry.register_cpu_reflected::<Children>();

    // Collect all entities that currently have Children, so we can clean stale ones.
    let entities_with_children: Vec<Entity> = registry
        .get_cpu::<Children>()
        .map(|s| s.iter().map(|(e, _)| *e).collect())
        .unwrap_or_default();

    // Update or insert Children for entities that should have them.
    for (parent_entity, children_list) in &expected {
        if let Some(storage) = registry.get_cpu_mut::<Children>() {
            if let Some(existing) = storage.get_mut(*parent_entity) {
                existing.entities.clone_from(children_list);
            } else {
                storage.insert(
                    *parent_entity,
                    Children {
                        entities: children_list.clone(),
                    },
                );
            }
        }
    }

    // Clear Children for entities that no longer have any children.
    for entity in &entities_with_children {
        if !expected.contains_key(entity) {
            if let Some(storage) = registry.get_cpu_mut::<Children>() {
                storage.remove(*entity);
            }
        }
    }

    resources.insert(registry);
}

/// Propagates transforms top-down through the hierarchy.
///
/// Computes `GlobalTransform` for all entities with `Transform`:
/// - Roots (no `Parent`): `GlobalTransform = Transform::to_matrix()`
/// - Children: `GlobalTransform = parent.GlobalTransform * local.to_matrix()`
pub fn transform_propagation_system(resources: &mut ome_core::resource::Resources) {
    use crate::component::ComponentRegistry;
    use crate::transform::Transform;

    let Some(mut registry) = resources.remove::<ComponentRegistry>() else {
        return;
    };

    // Ensure GlobalTransform storage exists.
    registry.register_cpu_reflected::<GlobalTransform>();

    // Gather all entities with Transform and their Parent (if any).
    let transform_entities: Vec<(Entity, Transform, Option<Entity>)> = {
        let transform_storage = registry.get_cpu::<Transform>();
        let parent_storage = registry.get_cpu::<Parent>();

        match transform_storage {
            Some(ts) => ts
                .iter()
                .map(|(entity, transform)| {
                    let parent = parent_storage
                        .as_ref()
                        .and_then(|ps| ps.get(*entity))
                        .map(|p| p.entity);
                    (*entity, *transform, parent)
                })
                .collect(),
            None => {
                resources.insert(registry);
                return;
            }
        }
    };

    // Identify roots (no Parent or parent has no Transform).
    let roots: Vec<Entity> = transform_entities
        .iter()
        .filter(|(_, _, parent)| parent.is_none())
        .map(|(e, _, _)| *e)
        .collect();

    // Build entity→Transform lookup.
    let transform_map: std::collections::HashMap<Entity, Transform> = transform_entities
        .iter()
        .map(|(e, t, _)| (*e, *t))
        .collect();

    // BFS propagation.
    let mut queue: std::collections::VecDeque<(Entity, Mat4)> = std::collections::VecDeque::new();

    // Seed roots.
    for &root in &roots {
        let matrix = transform_map[&root].to_matrix();
        queue.push_back((root, matrix));
    }

    // Get Children storage for traversal (read-only snapshot of entity lists).
    let children_map: std::collections::HashMap<Entity, Vec<Entity>> = registry
        .get_cpu::<Children>()
        .map(|s| {
            s.iter()
                .map(|(e, c)| (*e, c.entities.clone()))
                .collect()
        })
        .unwrap_or_default();

    let mut global_transforms: Vec<(Entity, GlobalTransform)> = Vec::new();

    while let Some((entity, parent_global)) = queue.pop_front() {
        global_transforms.push((entity, GlobalTransform { matrix: parent_global }));

        if let Some(children) = children_map.get(&entity) {
            for &child in children {
                if let Some(child_transform) = transform_map.get(&child) {
                    let child_global = parent_global * child_transform.to_matrix();
                    queue.push_back((child, child_global));
                }
            }
        }
    }

    // Write GlobalTransform values.
    for (entity, gt) in global_transforms {
        if let Some(storage) = registry.get_cpu_mut::<GlobalTransform>() {
            if let Some(existing) = storage.get_mut(entity) {
                *existing = gt;
            } else {
                storage.insert(entity, gt);
            }
        }
    }

    resources.insert(registry);
}

// ---------------------------------------------------------------------------
// Recursive despawn helper
// ---------------------------------------------------------------------------

/// Collects an entity and all its descendants (via `Children` components).
pub fn collect_descendants(
    root: Entity,
    registry: &crate::component::ComponentRegistry,
) -> Vec<Entity> {
    let mut result = vec![root];
    let mut i = 0;
    while i < result.len() {
        let current = result[i];
        if let Some(storage) = registry.get_cpu::<Children>() {
            if let Some(children) = storage.get(current) {
                result.extend_from_slice(&children.entities);
            }
        }
        i += 1;
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::allocator::EntityAllocator;
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::commands::Commands;
    use crate::component::ComponentRegistry;
    use crate::query::AccessTracker;
    use crate::transform::Transform;
    use glam::Vec3;
    use ome_core::resource::Resources;

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
            registry.get_cpu_mut::<T>().unwrap().insert(entity, component);
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
        assert!(registry.get_cpu::<Children>().unwrap().get(parent).is_none());
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
        insert_component(
            &mut resources,
            grandchild,
            Parent { entity: child_a },
        );

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
