//! Deferred command buffer for entity spawn/despawn and component operations.
//!
//! [`Commands`] collects mutation requests during system execution and applies
//! them in a single batch, preventing iterator invalidation and race conditions.
//!
//! Entity allocation is **immediate** (via [`EntityAllocator`]) so callers get
//! a valid [`Entity`] ID right away. Component insertion, removal, and despawn
//! are **deferred** until [`Commands::apply`] (or the built-in apply system).
//!
//! # Example
//!
//! ```ignore
//! fn setup_system(resources: &mut Resources) {
//!     let mut commands = Commands::new();
//!     let player = commands.spawn(resources)
//!         .insert(Health(100))
//!         .insert(Name("Player".into()))
//!         .id();
//!     commands.apply(resources);
//! }
//! ```

use std::any::TypeId;

use ome_core::resource::Resources;

use crate::allocator::EntityAllocator;
use crate::archetype::ArchetypeId;
use crate::archetype_registry::ArchetypeRegistry;
use crate::component::registry::ComponentRegistry;
use crate::component::traits::{Component, GpuComponent};
use crate::entity::Entity;

/// Type-erased function that inserts a component for an entity.
type InsertFn =
    Box<dyn FnOnce(Entity, &mut ComponentRegistry, &mut ArchetypeRegistry) + Send + Sync>;

/// A single deferred ECS command.
enum Command {
    /// Insert one or more components for an entity.
    InsertComponents {
        entity: Entity,
        inserts: Vec<InsertFn>,
    },
    /// Despawn an entity (mark dead, cleanup deferred to existing systems).
    Despawn(Entity),
    /// Remove a single component by TypeId.
    RemoveComponent { entity: Entity, type_id: TypeId },
}

/// Deferred command buffer for safe ECS mutations.
///
/// Collected commands are applied atomically via [`apply`](Commands::apply)
/// or the built-in [`commands_apply_system`].
pub struct Commands {
    queue: Vec<Command>,
}

impl Commands {
    /// Creates an empty command buffer.
    pub fn new() -> Self {
        Self { queue: Vec::new() }
    }

    /// Spawns a new entity and returns a builder for adding components.
    ///
    /// The entity ID is allocated **immediately** from [`EntityAllocator`]
    /// and registered in the empty archetype. Component insertions are
    /// deferred until [`apply`](Commands::apply).
    ///
    /// # Panics
    ///
    /// Panics if `EntityAllocator` or `ArchetypeRegistry` is missing from resources.
    pub fn spawn(&mut self, resources: &mut Resources) -> EntityBuilder<'_> {
        let entity = resources
            .get_mut::<EntityAllocator>()
            .expect("EntityAllocator not found in Resources")
            .spawn();
        resources
            .get_mut::<ArchetypeRegistry>()
            .expect("ArchetypeRegistry not found in Resources")
            .register_entity(entity, ArchetypeId::EMPTY);

        EntityBuilder {
            entity,
            inserts: Vec::new(),
            queue: &mut self.queue,
        }
    }

    /// Spawns multiple entities in a batch, calling `init` for each to
    /// provide its components via an [`EntityBuilder`].
    ///
    /// Returns the allocated entity IDs.
    pub fn spawn_batch(
        &mut self,
        resources: &mut Resources,
        count: usize,
        mut init: impl FnMut(usize, EntityBuilder<'_>),
    ) -> Vec<Entity> {
        let mut entities = Vec::with_capacity(count);
        for i in 0..count {
            let builder = self.spawn(resources);
            let entity = builder.entity;
            entities.push(entity);
            init(i, builder);
        }
        entities
    }

    /// Returns a builder for modifying an existing entity.
    ///
    /// Component insertions and removals are deferred until
    /// [`apply`](Commands::apply).
    pub fn entity(&mut self, entity: Entity) -> EntityCommands<'_> {
        EntityCommands {
            entity,
            inserts: Vec::new(),
            removals: Vec::new(),
            queue: &mut self.queue,
        }
    }

    /// Queues an entity for despawn.
    ///
    /// The actual deallocation happens during [`apply`](Commands::apply).
    /// Component cleanup is handled by the existing
    /// `component_despawn_cleanup_system`.
    pub fn despawn(&mut self, entity: Entity) {
        self.queue.push(Command::Despawn(entity));
    }

    /// Applies all queued commands to the ECS.
    ///
    /// Removes `ComponentRegistry`, `ArchetypeRegistry`, and `EntityAllocator`
    /// from resources temporarily, applies commands, then restores them.
    ///
    /// # Panics
    ///
    /// Panics if required resources are missing.
    pub fn apply(&mut self, resources: &mut Resources) {
        if self.queue.is_empty() {
            return;
        }

        let mut components = resources
            .remove::<ComponentRegistry>()
            .expect("ComponentRegistry not found");
        let mut archetypes = resources
            .remove::<ArchetypeRegistry>()
            .expect("ArchetypeRegistry not found");

        for command in self.queue.drain(..) {
            match command {
                Command::InsertComponents { entity, inserts } => {
                    for insert in inserts {
                        insert(entity, &mut components, &mut archetypes);
                    }
                }
                Command::Despawn(entity) => {
                    // Remove from archetype tracking.
                    archetypes.unregister_entity(entity);
                    // Remove all components.
                    components.remove_entity(entity);
                    // Mark slot as dead in allocator.
                    if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
                        alloc.despawn(entity);
                    }
                }
                Command::RemoveComponent { entity, type_id } => {
                    components.remove_component(entity, &type_id);
                    // Transition archetype.
                    if let Some(current) = archetypes.entity_archetype(entity) {
                        let new_arch =
                            archetypes.archetype_after_remove_dynamic(current, type_id);
                        archetypes.register_entity(entity, new_arch);
                    }
                }
            }
        }

        resources.insert(components);
        resources.insert(archetypes);
    }

    /// Returns `true` if there are no pending commands.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Returns the number of pending commands.
    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

impl Default for Commands {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for spawning a new entity with components.
///
/// Component insertions are committed to the command queue when this
/// builder is dropped (or when [`id`](EntityBuilder::id) is called).
pub struct EntityBuilder<'a> {
    entity: Entity,
    inserts: Vec<InsertFn>,
    queue: &'a mut Vec<Command>,
}

impl EntityBuilder<'_> {
    /// Adds a CPU component to the entity being spawned.
    pub fn insert<T: Component>(mut self, value: T) -> Self {
        self.inserts.push(Box::new(
            move |entity: Entity,
                  components: &mut ComponentRegistry,
                  archetypes: &mut ArchetypeRegistry| {
                components.register_cpu::<T>();
                components.get_cpu_mut::<T>().unwrap().insert(entity, value);
                let current = archetypes.entity_archetype(entity).unwrap();
                let new_arch = archetypes.archetype_after_add::<T>(current);
                archetypes.register_entity(entity, new_arch);
            },
        ));
        self
    }

    /// Adds a GPU component to the entity being spawned.
    ///
    /// The GPU buffer label is derived from the type name automatically.
    pub fn insert_gpu<T: GpuComponent>(mut self, value: T) -> Self {
        self.inserts.push(Box::new(
            move |entity: Entity,
                  components: &mut ComponentRegistry,
                  archetypes: &mut ArchetypeRegistry| {
                components.register_gpu::<T>(std::any::type_name::<T>());
                components.get_gpu_mut::<T>().unwrap().insert(entity, value);
                let current = archetypes.entity_archetype(entity).unwrap();
                let new_arch = archetypes.archetype_after_add::<T>(current);
                archetypes.register_entity(entity, new_arch);
            },
        ));
        self
    }

    /// Finalises the builder and returns the pre-allocated entity ID.
    pub fn id(self) -> Entity {
        self.entity
    }
}

impl Drop for EntityBuilder<'_> {
    fn drop(&mut self) {
        let inserts = std::mem::take(&mut self.inserts);
        if !inserts.is_empty() {
            self.queue.push(Command::InsertComponents {
                entity: self.entity,
                inserts,
            });
        }
    }
}

/// Builder for modifying an existing entity.
///
/// Operations are committed to the command queue on drop.
pub struct EntityCommands<'a> {
    entity: Entity,
    inserts: Vec<InsertFn>,
    removals: Vec<TypeId>,
    queue: &'a mut Vec<Command>,
}

impl EntityCommands<'_> {
    /// Adds a CPU component to the entity.
    pub fn insert<T: Component>(&mut self, value: T) -> &mut Self {
        self.inserts.push(Box::new(
            move |entity: Entity,
                  components: &mut ComponentRegistry,
                  archetypes: &mut ArchetypeRegistry| {
                components.register_cpu::<T>();
                components.get_cpu_mut::<T>().unwrap().insert(entity, value);
                let current = archetypes.entity_archetype(entity).unwrap();
                let new_arch = archetypes.archetype_after_add::<T>(current);
                archetypes.register_entity(entity, new_arch);
            },
        ));
        self
    }

    /// Adds a GPU component to the entity.
    pub fn insert_gpu<T: GpuComponent>(&mut self, value: T) -> &mut Self {
        self.inserts.push(Box::new(
            move |entity: Entity,
                  components: &mut ComponentRegistry,
                  archetypes: &mut ArchetypeRegistry| {
                components.register_gpu::<T>(std::any::type_name::<T>());
                components.get_gpu_mut::<T>().unwrap().insert(entity, value);
                let current = archetypes.entity_archetype(entity).unwrap();
                let new_arch = archetypes.archetype_after_add::<T>(current);
                archetypes.register_entity(entity, new_arch);
            },
        ));
        self
    }

    /// Removes a component type from the entity.
    pub fn remove<T: 'static>(&mut self) -> &mut Self {
        self.removals.push(TypeId::of::<T>());
        self
    }

    /// Queues the entity for despawn.
    pub fn despawn(self) {
        // Skip the Drop impl — despawn replaces all pending inserts/removals.
        let entity = self.entity;
        let queue = unsafe { &mut *(self.queue as *mut Vec<Command>) };
        std::mem::forget(self);
        queue.push(Command::Despawn(entity));
    }
}

impl Drop for EntityCommands<'_> {
    fn drop(&mut self) {
        let inserts = std::mem::take(&mut self.inserts);
        if !inserts.is_empty() {
            self.queue.push(Command::InsertComponents {
                entity: self.entity,
                inserts,
            });
        }
        for type_id in std::mem::take(&mut self.removals) {
            self.queue.push(Command::RemoveComponent {
                entity: self.entity,
                type_id,
            });
        }
    }
}

/// System that applies all pending [`Commands`].
///
/// Should run in [`Stage::GpuSync`](ome_core::stage::Stage::GpuSync) **before**
/// the despawn cleanup system so that newly spawned entities get their
/// components before GPU sync.
pub fn commands_apply_system(resources: &mut Resources) {
    let mut commands = match resources.remove::<Commands>() {
        Some(c) => c,
        None => return,
    };
    commands.apply(resources);
    resources.insert(commands);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{Query, With, Without};

    // -- Test components --

    struct Health(u32);
    impl Component for Health {}

    struct Name(String);
    impl Component for Name {}

    struct Marker;
    impl Component for Marker {}

    #[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
    #[repr(C)]
    struct Position {
        x: f32,
        y: f32,
    }
    impl GpuComponent for Position {}

    fn setup() -> Resources {
        let mut resources = Resources::new();
        resources.insert(EntityAllocator::new());
        resources.insert(ComponentRegistry::new());
        resources.insert(ArchetypeRegistry::new());
        resources.insert(crate::query::AccessTracker::new());
        resources
    }

    // -- Spawn tests --

    #[test]
    fn spawn_single_entity() {
        let mut resources = setup();
        let mut commands = Commands::new();

        let entity = commands
            .spawn(&mut resources)
            .insert(Health(100))
            .id();

        assert!(entity.is_valid());
        commands.apply(&mut resources);

        let query = Query::<&Health>::new(&resources);
        let health = query.get(entity).unwrap();
        assert_eq!(health.0, 100);
    }

    #[test]
    fn spawn_multiple_components() {
        let mut resources = setup();
        let mut commands = Commands::new();

        let entity = commands
            .spawn(&mut resources)
            .insert(Health(100))
            .insert(Name("Player".into()))
            .id();

        commands.apply(&mut resources);

        let query = Query::<(&Health, &Name)>::new(&resources);
        let (health, name) = query.get(entity).unwrap();
        assert_eq!(health.0, 100);
        assert_eq!(name.0, "Player");
    }

    #[test]
    fn spawn_with_gpu_component() {
        let mut resources = setup();
        let mut commands = Commands::new();

        let entity = commands
            .spawn(&mut resources)
            .insert(Health(100))
            .insert_gpu(Position { x: 1.0, y: 2.0 })
            .id();

        commands.apply(&mut resources);

        let query = Query::<(&Health, &Position)>::new(&resources);
        let (health, pos) = query.get(entity).unwrap();
        assert_eq!(health.0, 100);
        assert_eq!(pos.x, 1.0);
        assert_eq!(pos.y, 2.0);
    }

    #[test]
    fn spawn_without_id() {
        let mut resources = setup();
        let mut commands = Commands::new();

        // Just drop the builder — entity still gets created.
        commands
            .spawn(&mut resources)
            .insert(Health(42));

        commands.apply(&mut resources);

        let query = Query::<&Health>::new(&resources);
        let health = query.iter().next().unwrap();
        assert_eq!(health.0, 42);
    }

    #[test]
    fn spawn_batch_entities() {
        let mut resources = setup();
        let mut commands = Commands::new();

        let entities = commands.spawn_batch(&mut resources, 5, |i, builder| {
            builder.insert(Health(i as u32 * 10));
        });

        assert_eq!(entities.len(), 5);
        commands.apply(&mut resources);

        let query = Query::<&Health>::new(&resources);
        let total: u32 = query.iter().map(|h| h.0).sum();
        assert_eq!(total, 0 + 10 + 20 + 30 + 40);
    }

    // -- Despawn tests --

    #[test]
    fn despawn_entity() {
        let mut resources = setup();
        let mut commands = Commands::new();

        let entity = commands
            .spawn(&mut resources)
            .insert(Health(100))
            .id();
        commands.apply(&mut resources);

        // Verify entity exists.
        assert!(Query::<&Health>::new(&resources).get(entity).is_some());

        // Despawn.
        commands.despawn(entity);
        commands.apply(&mut resources);

        // Entity should be gone.
        let query = Query::<&Health>::new(&resources);
        assert!(query.get(entity).is_none());
        assert!(query.is_empty());
    }

    #[test]
    fn despawn_via_entity_commands() {
        let mut resources = setup();
        let mut commands = Commands::new();

        let entity = commands
            .spawn(&mut resources)
            .insert(Health(100))
            .id();
        commands.apply(&mut resources);

        commands.entity(entity).despawn();
        commands.apply(&mut resources);

        assert!(Query::<&Health>::new(&resources).is_empty());
    }

    // -- Insert/Remove on existing entity --

    #[test]
    fn insert_component_on_existing_entity() {
        let mut resources = setup();
        let mut commands = Commands::new();

        let entity = commands.spawn(&mut resources).insert(Health(100)).id();
        commands.apply(&mut resources);

        // Add a new component later.
        commands.entity(entity).insert(Name("Updated".into()));
        commands.apply(&mut resources);

        let query = Query::<(&Health, &Name)>::new(&resources);
        let (health, name) = query.get(entity).unwrap();
        assert_eq!(health.0, 100);
        assert_eq!(name.0, "Updated");
    }

    #[test]
    fn remove_component_from_entity() {
        let mut resources = setup();
        let mut commands = Commands::new();

        let entity = commands
            .spawn(&mut resources)
            .insert(Health(100))
            .insert(Marker)
            .id();
        commands.apply(&mut resources);

        // Remove Marker component.
        commands.entity(entity).remove::<Marker>();
        commands.apply(&mut resources);

        // Entity should still have Health but not Marker.
        let query = Query::<&Health, Without<Marker>>::new(&resources);
        assert!(query.get(entity).is_some());

        let query = Query::<&Health, With<Marker>>::new(&resources);
        assert!(query.get(entity).is_none());
    }

    // -- Apply system --

    #[test]
    fn commands_apply_system_works() {
        let mut resources = setup();
        resources.insert(Commands::new());

        // Spawn via the resource.
        {
            let mut commands = resources.remove::<Commands>().unwrap();
            commands.spawn(&mut resources).insert(Health(77));
            resources.insert(commands);
        }

        commands_apply_system(&mut resources);

        let query = Query::<&Health>::new(&resources);
        assert_eq!(query.iter().next().unwrap().0, 77);
    }

    #[test]
    fn empty_commands_apply_is_noop() {
        let mut resources = setup();
        let mut commands = Commands::new();
        commands.apply(&mut resources); // Should not panic.
    }

    // -- Edge cases --

    #[test]
    fn spawn_and_despawn_same_batch() {
        let mut resources = setup();
        let mut commands = Commands::new();

        let entity = commands.spawn(&mut resources).insert(Health(100)).id();
        commands.despawn(entity);
        commands.apply(&mut resources);

        // Entity was spawned then immediately despawned.
        assert!(Query::<&Health>::new(&resources).is_empty());
    }

    #[test]
    fn multiple_apply_calls() {
        let mut resources = setup();
        let mut commands = Commands::new();

        let e1 = commands.spawn(&mut resources).insert(Health(10)).id();
        commands.apply(&mut resources);

        let e2 = commands.spawn(&mut resources).insert(Health(20)).id();
        commands.apply(&mut resources);

        let query = Query::<&Health>::new(&resources);
        assert_eq!(query.get(e1).unwrap().0, 10);
        assert_eq!(query.get(e2).unwrap().0, 20);
    }
}
