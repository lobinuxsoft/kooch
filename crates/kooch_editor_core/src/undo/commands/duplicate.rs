//! [`DuplicateCommand`] — clones a source entity's component set into
//! a fresh entity, preserving every reflected field value.
//!
//! The destination entity gets a brand-new [`Entity`] handle (no slot
//! revival like Spawn). Component types are read from the source's
//! archetype; per-component field values are snapshotted via the
//! reflect registry and re-applied on the new entity. Non-reflected
//! components are added with their default value (the same as
//! `SpawnCommand` does for extras).
//!
//! Undo despawns the duplicated entity. Redo re-runs the duplication
//! against a freshly-allocated handle.

use std::any::TypeId;
use std::collections::BTreeSet;

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;

use crate::undo::{ComponentSnapshot, EditorCommand};

pub(crate) struct DuplicateCommand {
    /// Source entity to copy from. Resolved to a list of components +
    /// snapshots at construction time so the undo history is stable
    /// even if the source changes later.
    component_types: BTreeSet<TypeId>,
    snapshots: Vec<ComponentSnapshot>,
    /// Destination entity, populated on the first execute. Reused on
    /// redo to keep undo history coherent.
    duplicate: Option<Entity>,
}

impl DuplicateCommand {
    pub fn new(resources: &Resources, source: Entity) -> Self {
        let mut component_types = BTreeSet::new();
        let mut snapshots = Vec::new();

        if let Some(archetypes) = resources.get::<ArchetypeRegistry>() {
            if let Some(arch_id) = archetypes.entity_archetype(source) {
                if let Some(arch) = archetypes.get(arch_id) {
                    component_types = arch.components().clone();
                }
            }
        }

        if let Some(registry) = resources.get::<ComponentRegistry>() {
            for &type_id in &component_types {
                if let Some(fields) = registry.reflect_get_fields(&type_id, source) {
                    snapshots.push(ComponentSnapshot { type_id, fields });
                }
            }
        }

        Self {
            component_types,
            snapshots,
            duplicate: None,
        }
    }

    fn do_duplicate(&mut self, resources: &mut Resources) {
        let entity = if let Some(existing) = self.duplicate {
            // Redo path: revive the same handle so the undo history
            // remains stable across undo/redo cycles.
            let revived = resources
                .get_mut::<EntityAllocator>()
                .is_some_and(|alloc| alloc.revive(existing));
            if !revived {
                tracing::warn!(
                    "redo Duplicate: failed to revive entity {existing}; allocating fresh",
                );
                self.allocate_fresh(resources)
            } else {
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    archetypes.register_entity(existing, kooch_ecs::archetype::ArchetypeId::EMPTY);
                }
                existing
            }
        } else {
            self.allocate_fresh(resources)
        };
        self.duplicate = Some(entity);

        // Add each component type. insert_default_reflected covers
        // reflected types; non-reflected types are silently skipped
        // (matches SpawnCommand's contract). Archetype is advanced
        // after each successful insertion.
        for type_id in &self.component_types {
            let inserted = resources
                .get_mut::<ComponentRegistry>()
                .is_some_and(|registry| registry.insert_default_reflected(type_id, entity));
            if inserted {
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(entity) {
                        let new_arch = archetypes.archetype_after_add_dynamic(current, *type_id);
                        archetypes.register_entity(entity, new_arch);
                    }
                }
            }
        }

        // Restore field values from the source snapshots.
        for snapshot in &self.snapshots {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                for (field, value) in &snapshot.fields {
                    if let Err(e) =
                        registry.reflect_set_field(&snapshot.type_id, entity, field, value.clone())
                    {
                        tracing::warn!("duplicate: failed to restore field '{field}': {e}",);
                    }
                }
            }
        }
    }

    fn allocate_fresh(&self, resources: &mut Resources) -> Entity {
        use kooch_ecs::commands::Commands;
        let mut commands = resources.remove::<Commands>().expect("Commands not found");
        let entity = commands.spawn(resources).id();
        resources.insert(commands);
        entity
    }
}

impl EditorCommand for DuplicateCommand {
    fn execute(&mut self, resources: &mut Resources) {
        self.do_duplicate(resources);
    }

    fn undo(&mut self, resources: &mut Resources) {
        let Some(entity) = self.duplicate else { return };
        if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
            alloc.despawn(entity);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.unregister_entity(entity);
        }
        if let Some(components) = resources.get_mut::<ComponentRegistry>() {
            components.remove_entity(entity);
        }
    }

    fn description(&self) -> &str {
        "Duplicate Entity"
    }
}
