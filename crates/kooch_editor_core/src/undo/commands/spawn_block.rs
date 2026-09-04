//! [`SpawnBlockCommand`] — a new block, asset and entity together.
//!
//! Writes a fresh `.blockmesh.ron` holding a cube, registers it so it
//! has a [`Guid`], then spawns an entity carrying `Name`, `Transform`,
//! `Block`, `MeshRenderer` and `Collider`, pointing at it.
//! `sync_blocks` fills the last two in from the source on the next
//! frame.
//!
//! One command rather than two because a block's shape belongs to the
//! entity standing in the level: sharing a source means editing either
//! one moves both, which is occasionally wanted and never the default.

use std::any::TypeId;
use std::path::PathBuf;

use kooch_blockmesh::Block;
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::reflect::ReflectValue;
use kooch_physics::components::Collider;

use crate::undo::EditorCommand;

pub(crate) struct SpawnBlockCommand {
    into: crate::actions::SpawnTarget,
    /// Entity allocated on first execute, reused on redo.
    entity: Option<Entity>,
    /// The source written on first execute. Kept so redo points at the
    /// same file rather than writing a second one.
    source: Option<Guid>,
    /// Where that file went, for the log and for undo to say so.
    path: Option<PathBuf>,
    spawned_component_types: Vec<TypeId>,
}

impl SpawnBlockCommand {
    pub fn new(into: crate::actions::SpawnTarget) -> Self {
        Self {
            into,
            entity: None,
            source: None,
            path: None,
            spawned_component_types: Vec::new(),
        }
    }

    /// Writes the cube and registers it, once. Redo reuses the answer.
    fn ensure_source(&mut self, resources: &mut Resources) -> Option<Guid> {
        if let Some(guid) = self.source {
            return Some(guid);
        }
        let (file, guid) = crate::actions::asset_ops::new_block_asset(resources)?;
        self.path = Some(file);
        self.source = Some(guid);
        Some(guid)
    }

    fn spawn_fresh_entity(&self, resources: &mut Resources) -> Entity {
        use kooch_ecs::commands::Commands;
        let mut commands = resources
            .remove::<Commands>()
            .expect("Commands resource missing");
        let entity = commands.spawn(resources).id();
        resources.insert(commands);
        entity
    }

    fn do_spawn(&mut self, resources: &mut Resources) {
        let source = self.ensure_source(resources);

        let entity = match self.entity {
            Some(existing) => match revive(resources, existing) {
                true => existing,
                false => self.spawn_fresh_entity(resources),
            },
            None => self.spawn_fresh_entity(resources),
        };
        self.entity = Some(entity);

        let mut types = named_types(resources, &["Name", "Transform"]);
        types.push(TypeId::of::<Block>());
        types.push(TypeId::of::<MeshRenderer>());
        types.push(TypeId::of::<Collider>());

        for type_id in &types {
            let inserted = resources
                .get_mut::<ComponentRegistry>()
                .is_some_and(|registry| registry.insert_default_reflected(type_id, entity));
            if inserted
                && let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>()
                && let Some(current) = archetypes.entity_archetype(entity)
            {
                let after = archetypes.archetype_after_add_dynamic(current, *type_id);
                archetypes.register_entity(entity, after);
            }
        }
        self.spawned_component_types = types;

        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            let _ = registry.reflect_set_field(
                &TypeId::of::<kooch_ecs::Name>(),
                entity,
                "value",
                ReflectValue::String("Block".to_owned()),
            );
        }

        // Only the source is written. The renderer's mesh and the
        // collider's are `sync_blocks`'s answer, and writing them here
        // would be a second place that decides what a block draws.
        if let Some(guid) = source
            && let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<Block>()
            && let Some(block) = storage.get_mut(entity)
        {
            block.source = Some(guid);
        }

        self.place(resources, entity);
    }

    /// Puts the block where the menu asked for it, rather than in
    /// whichever scene happens to be active — see `SpawnCommand::place`.
    fn place(&self, resources: &mut Resources, entity: Entity) {
        use crate::actions::SpawnTarget;

        if let SpawnTarget::ChildOf(parent) = self.into {
            kooch_ecs::hierarchy::reparent(resources, entity, Some(parent));
        }
        if let Some(scene) = super::place::resolve_scene(resources, self.into) {
            super::place::adopt(resources, entity, scene);
        }
    }
}

/// The reflected type ids behind a list of short component names.
fn named_types(resources: &Resources, wanted: &[&str]) -> Vec<TypeId> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let names = registry.reflected_type_names();
    wanted
        .iter()
        .filter_map(|needle| {
            names
                .iter()
                .find(|(_, name)| name.rsplit("::").next().unwrap_or(name) == *needle)
                .map(|(type_id, _)| *type_id)
        })
        .collect()
}

/// Brings an undone entity back, or says it could not.
fn revive(resources: &mut Resources, entity: Entity) -> bool {
    let revived = resources
        .get_mut::<EntityAllocator>()
        .is_some_and(|alloc| alloc.revive(entity));
    if revived && let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
        archetypes.register_entity(entity, kooch_ecs::archetype::ArchetypeId::EMPTY);
    }
    revived
}

impl EditorCommand for SpawnBlockCommand {
    fn execute(&mut self, resources: &mut Resources) {
        self.do_spawn(resources);
    }

    /// Removes the entity and leaves the file.
    ///
    /// Deleting it would take a shape somebody may have already edited,
    /// and an orphaned cube in `assets/blocks` is visible and cheap. A
    /// redo finds it again by GUID rather than writing a second one.
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
        "Spawn Block"
    }
}
