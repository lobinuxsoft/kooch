//! Hierarchy sync system — keeps Parent ↔ Children consistent.

use std::any::TypeId;
use std::collections::HashMap;

use crate::archetype_registry::ArchetypeRegistry;
use crate::entity::Entity;

use super::children::Children;
use super::parent::Parent;

/// Synchronizes `Parent` ↔ `Children` components.
///
/// Runs in [`Stage::PostUpdate`]. `Parent` is authoritative:
/// - If entity A has `Parent(B)`, A is added to B's `Children`.
/// - Stale entries in `Children` (where the child's `Parent` doesn't match)
///   are removed.
///
/// Also keeps archetypes in sync: entities that gain or lose `Children`
/// have their archetype updated accordingly.
pub fn hierarchy_sync_system(resources: &mut kooch_core::resource::Resources) {
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
    let mut expected: HashMap<Entity, Vec<Entity>> = HashMap::new();
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

    // Track which entities gained or lost Children for archetype sync.
    let mut gained_children: Vec<Entity> = Vec::new();
    let mut lost_children: Vec<Entity> = Vec::new();

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
                gained_children.push(*parent_entity);
            }
        }
    }

    // Clear Children for entities that no longer have any children.
    for entity in &entities_with_children {
        if !expected.contains_key(entity) {
            if let Some(storage) = registry.get_cpu_mut::<Children>() {
                storage.remove(*entity);
            }
            lost_children.push(*entity);
        }
    }

    // Restore ComponentRegistry before archetype updates.
    resources.insert(registry);

    // -- Phase 3: Sync archetypes for entities that gained/lost Children.
    let children_tid = TypeId::of::<Children>();

    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        for entity in gained_children {
            if let Some(current) = archetypes.entity_archetype(entity) {
                let new_arch = archetypes.archetype_after_add_dynamic(current, children_tid);
                archetypes.register_entity(entity, new_arch);
            }
        }
        for entity in lost_children {
            if let Some(current) = archetypes.entity_archetype(entity) {
                let new_arch = archetypes.archetype_after_remove_dynamic(current, children_tid);
                archetypes.register_entity(entity, new_arch);
            }
        }
    }
}
