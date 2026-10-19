//! [`SpawnBlockCommand`] — a new block, geometry and entity together.

use std::any::TypeId;

use kooch_blockmesh::Block;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::ReflectValue;

use crate::undo::EditorCommand;

pub(crate) struct SpawnBlockCommand {
    into: crate::actions::SpawnTarget,
    shape: kooch_blockmesh::Shape,
    /// Entity allocated on first execute, reused on redo.
    entity: Option<Entity>,
    spawned_component_types: Vec<TypeId>,
    /// Where in the world it lands.
    at: crate::viewport_pick::DropPoint,
    /// The resolved world point, kept so a redo lands where the first spawn did.
    placed: Option<glam::Vec3>,
}

impl SpawnBlockCommand {
    pub fn new(
        into: crate::actions::SpawnTarget,
        shape: kooch_blockmesh::Shape,
        at: crate::viewport_pick::DropPoint,
    ) -> Self {
        Self {
            into,
            shape,
            at,
            placed: None,
            entity: None,
            spawned_component_types: Vec::new(),
        }
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
        let entity = match self.entity {
            Some(existing) => match revive(resources, existing) {
                true => existing,
                false => self.spawn_fresh_entity(resources),
            },
            None => self.spawn_fresh_entity(resources),
        };
        self.entity = Some(entity);

        let mut types = named_types(resources, &["Name", "Transform"]);
        types.extend(kooch_blockmesh::block_components().map(|(type_id, _)| type_id));
        types.push(TypeId::of::<kooch_blockmesh::BlockShape>());

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
                ReflectValue::String(self.shape.label().to_owned()),
            );
        }

        // Static, because a wall that falls over is not a level. The
        // author can make it dynamic; nothing can make a body that was
        // never authored.
        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<kooch_physics::components::PhysicsBody>()
            && let Some(body) = storage.get_mut(entity)
        {
            body.kind = kooch_physics::components::KIND_STATIC;
        }

        // Only the geometry is written. The renderer's mesh and the collider's are
        // `sync_blocks`'s answer, and writing them here would be a second place that decides
        // what a block draws.
        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<Block>()
            && let Some(block) = storage.get_mut(entity)
        {
            block.mesh = self.shape.build();
        }

        // The parameters stay on the block, so the Inspector can reshape it until it is edited by hand.
        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(storage) = registry.get_cpu_mut::<kooch_blockmesh::BlockShape>()
            && let Some(shape) = storage.get_mut(entity)
        {
            *shape = kooch_blockmesh::BlockShape::from(self.shape);
        }

        self.place(resources, entity);
        super::place::drop_at(resources, entity, self.at, &mut self.placed);
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

/// The engine's own prototype grid, if it is registered.
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
