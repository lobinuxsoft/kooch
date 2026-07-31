//! [`SpawnCommand`] — spawns a new entity with Name + Transform plus
//! optional extra components. Undo despawns it; redo revives the same
//! entity handle.

use std::any::TypeId;

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::ReflectValue;

use crate::undo::EditorCommand;

pub(crate) struct SpawnCommand {
    /// The entity that was (or will be) spawned.
    entity: Option<Entity>,
    /// Extra component TypeIds to add beyond Name + Transform.
    extra_component_types: Vec<TypeId>,
    /// Optional name to set on the Name component.
    name: Option<String>,
    /// All component TypeIds added during spawn (base + extra).
    spawned_component_types: Vec<TypeId>,
}

impl SpawnCommand {
    pub fn new(extra_component_types: Vec<TypeId>, name: Option<String>) -> Self {
        Self {
            entity: None,
            extra_component_types,
            name,
            spawned_component_types: Vec::new(),
        }
    }

    /// Performs the actual spawn logic (shared between execute and redo).
    fn do_spawn(&mut self, resources: &mut Resources) {
        // Allocate entity — either revive or fresh spawn.
        let entity = if let Some(e) = self.entity {
            // Redo path: try to revive the same entity.
            if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
                if alloc.revive(e) {
                    // Re-register into EMPTY archetype.
                    if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                        archetypes.register_entity(e, kooch_ecs::archetype::ArchetypeId::EMPTY);
                    }
                    e
                } else {
                    tracing::warn!("undo: failed to revive entity {e}, spawning new");
                    self.spawn_fresh(resources)
                }
            } else {
                return;
            }
        } else {
            // First execute.
            self.spawn_fresh(resources)
        };

        self.entity = Some(entity);

        // Auto-add base components in deterministic order: Name, Transform, then extras.
        let mut all_types: Vec<TypeId> = Vec::new();
        if let Some(reg) = resources.get::<ComponentRegistry>() {
            let type_names = reg.reflected_type_names();
            // Name first.
            if let Some((tid, _)) = type_names
                .iter()
                .find(|(_, name)| name.rsplit("::").next().unwrap_or(name) == "Name")
            {
                all_types.push(*tid);
            }
            // Transform second.
            if let Some((tid, _)) = type_names
                .iter()
                .find(|(_, name)| name.rsplit("::").next().unwrap_or(name) == "Transform")
            {
                all_types.push(*tid);
            }
        }

        // Append extra component types (avoid duplicates).
        for tid in &self.extra_component_types {
            if !all_types.contains(tid) {
                all_types.push(*tid);
            }
        }

        for type_id in &all_types {
            let mut inserted = false;
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                inserted = registry.insert_default_reflected(type_id, entity);
            }
            if inserted {
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(entity) {
                        let new_arch = archetypes.archetype_after_add_dynamic(current, *type_id);
                        archetypes.register_entity(entity, new_arch);
                    }
                }
            }
        }
        self.spawned_component_types = all_types;

        // Set the Name component value if provided.
        if let Some(ref name) = self.name {
            let name_tid = TypeId::of::<kooch_ecs::Name>();
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                let _ = registry.reflect_set_field(
                    &name_tid,
                    entity,
                    "value",
                    ReflectValue::String(name.clone()),
                );
            }
        }
    }

    fn spawn_fresh(&self, resources: &mut Resources) -> Entity {
        use kooch_ecs::commands::Commands;
        let mut commands = resources.remove::<Commands>().expect("Commands not found");
        let entity = commands.spawn(resources).id();
        resources.insert(commands);
        entity
    }
}

impl EditorCommand for SpawnCommand {
    fn execute(&mut self, resources: &mut Resources) {
        self.do_spawn(resources);
    }

    fn undo(&mut self, resources: &mut Resources) {
        let Some(entity) = self.entity else { return };

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
        "Spawn Entity"
    }
}
