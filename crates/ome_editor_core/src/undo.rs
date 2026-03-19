//! Undo/redo system using the command pattern.
//!
//! Each undoable editor operation is wrapped in an [`EditorCommand`] that
//! captures before-state on construction. The [`UndoStack`] resource holds
//! the command history and provides `undo()`/`redo()` operations.

use std::any::TypeId;
use std::collections::BTreeSet;

use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::ReflectValue;

/// A snapshot of all reflected field values for a single component on an entity.
#[derive(Debug, Clone)]
pub(crate) struct ComponentSnapshot {
    pub type_id: TypeId,
    pub fields: Vec<(String, ReflectValue)>,
}

// ---------------------------------------------------------------------------
// EditorCommand trait
// ---------------------------------------------------------------------------

/// A reversible editor operation.
pub(crate) trait EditorCommand: Send + Sync {
    /// Apply the command (first time or redo).
    fn execute(&mut self, resources: &mut Resources);
    /// Reverse the command.
    fn undo(&mut self, resources: &mut Resources);
    /// Human-readable description for UI display.
    fn description(&self) -> &str;
}

// ---------------------------------------------------------------------------
// UndoStack resource
// ---------------------------------------------------------------------------

/// History of undoable commands. Registered as a resource in the editor.
pub(crate) struct UndoStack {
    undo_stack: Vec<Box<dyn EditorCommand>>,
    redo_stack: Vec<Box<dyn EditorCommand>>,
    max_history: usize,
}

impl UndoStack {
    pub fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_history: 100,
        }
    }

    /// Executes a command and pushes it onto the undo stack.
    pub fn execute(&mut self, mut cmd: Box<dyn EditorCommand>, resources: &mut Resources) {
        cmd.execute(resources);
        self.undo_stack.push(cmd);
        self.redo_stack.clear();

        // Trim oldest entries if over capacity.
        while self.undo_stack.len() > self.max_history {
            self.undo_stack.remove(0);
        }
    }

    /// Undoes the most recent command.
    pub fn undo(&mut self, resources: &mut Resources) {
        if let Some(mut cmd) = self.undo_stack.pop() {
            cmd.undo(resources);
            self.redo_stack.push(cmd);
        }
    }

    /// Redoes the most recently undone command.
    pub fn redo(&mut self, resources: &mut Resources) {
        if let Some(mut cmd) = self.redo_stack.pop() {
            cmd.execute(resources);
            self.undo_stack.push(cmd);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Clears all history (e.g. on scene load).
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Returns the description of the command that would be undone next.
    pub fn undo_description(&self) -> Option<&str> {
        self.undo_stack.last().map(|cmd| cmd.description())
    }

    /// Returns the description of the command that would be redone next.
    pub fn redo_description(&self) -> Option<&str> {
        self.redo_stack.last().map(|cmd| cmd.description())
    }
}

// ---------------------------------------------------------------------------
// CompoundCommand — groups multiple commands as a single undo/redo step
// ---------------------------------------------------------------------------

/// Groups multiple commands into a single atomic undo/redo operation.
pub(crate) struct CompoundCommand {
    commands: Vec<Box<dyn EditorCommand>>,
    desc: String,
}

impl CompoundCommand {
    pub fn new(desc: impl Into<String>, commands: Vec<Box<dyn EditorCommand>>) -> Self {
        Self {
            commands,
            desc: desc.into(),
        }
    }
}

impl EditorCommand for CompoundCommand {
    fn execute(&mut self, resources: &mut Resources) {
        for cmd in &mut self.commands {
            cmd.execute(resources);
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        // Undo in reverse order.
        for cmd in self.commands.iter_mut().rev() {
            cmd.undo(resources);
        }
    }

    fn description(&self) -> &str {
        &self.desc
    }
}

// ---------------------------------------------------------------------------
// SetFieldCommand
// ---------------------------------------------------------------------------

pub(crate) struct SetFieldCommand {
    entity: Entity,
    type_id: TypeId,
    field: String,
    new_value: ReflectValue,
    old_value: ReflectValue,
}

impl SetFieldCommand {
    /// Creates the command, capturing the old field value from the registry.
    ///
    /// Returns `None` if the component/field doesn't exist.
    pub fn new(
        resources: &Resources,
        entity: Entity,
        type_id: TypeId,
        field: String,
        new_value: ReflectValue,
    ) -> Option<Self> {
        let registry = resources.get::<ComponentRegistry>()?;
        let fields = registry.reflect_get_fields(&type_id, entity)?;
        let old_value = fields
            .into_iter()
            .find(|(name, _)| name == &field)
            .map(|(_, v)| v)?;
        Some(Self {
            entity,
            type_id,
            field,
            new_value,
            old_value,
        })
    }
}

impl EditorCommand for SetFieldCommand {
    fn execute(&mut self, resources: &mut Resources) {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            if let Err(e) = registry.reflect_set_field(
                &self.type_id,
                self.entity,
                &self.field,
                self.new_value.clone(),
            ) {
                tracing::warn!("undo: failed to set field '{}': {e}", self.field);
            }
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            if let Err(e) = registry.reflect_set_field(
                &self.type_id,
                self.entity,
                &self.field,
                self.old_value.clone(),
            ) {
                tracing::warn!("undo: failed to restore field '{}': {e}", self.field);
            }
        }
    }

    fn description(&self) -> &str {
        "Set Field"
    }
}

// ---------------------------------------------------------------------------
// SpawnCommand
// ---------------------------------------------------------------------------

pub(crate) struct SpawnCommand {
    /// The entity that was (or will be) spawned.
    entity: Option<Entity>,
    /// Component TypeIds added during spawn (Name, Transform defaults).
    default_component_types: Vec<TypeId>,
}

impl SpawnCommand {
    pub fn new() -> Self {
        Self {
            entity: None,
            default_component_types: Vec::new(),
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
                        archetypes
                            .register_entity(e, ome_ecs::archetype::ArchetypeId::EMPTY);
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

        // Auto-add Name and Transform defaults.
        let default_types: Vec<TypeId> = resources
            .get::<ComponentRegistry>()
            .map(|reg| {
                reg.reflected_type_names()
                    .into_iter()
                    .filter(|(_, name)| {
                        let short = name.rsplit("::").next().unwrap_or(name);
                        short == "Name" || short == "Transform"
                    })
                    .map(|(tid, _)| tid)
                    .collect()
            })
            .unwrap_or_default();

        for type_id in &default_types {
            let mut inserted = false;
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                inserted = registry.insert_default_reflected(type_id, entity);
            }
            if inserted {
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(entity) {
                        let new_arch =
                            archetypes.archetype_after_add_dynamic(current, *type_id);
                        archetypes.register_entity(entity, new_arch);
                    }
                }
            }
        }
        self.default_component_types = default_types;
    }

    fn spawn_fresh(&self, resources: &mut Resources) -> Entity {
        use ome_ecs::commands::Commands;
        let mut commands = resources
            .remove::<Commands>()
            .expect("Commands not found");
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

// ---------------------------------------------------------------------------
// DespawnCommand
// ---------------------------------------------------------------------------

pub(crate) struct DespawnCommand {
    entity: Entity,
    /// Snapshots of all reflected components, captured before despawn.
    snapshots: Vec<ComponentSnapshot>,
    /// All component TypeIds the entity had (including non-reflected).
    component_types: BTreeSet<TypeId>,
}

impl DespawnCommand {
    /// Creates the command, snapshotting the entity's reflected component state.
    pub fn new(resources: &Resources, entity: Entity) -> Self {
        let mut snapshots = Vec::new();
        let mut component_types = BTreeSet::new();

        // Get the entity's archetype to know which components it has.
        if let Some(archetypes) = resources.get::<ArchetypeRegistry>() {
            if let Some(arch_id) = archetypes.entity_archetype(entity) {
                if let Some(arch) = archetypes.get(arch_id) {
                    component_types = arch.components().clone();
                }
            }
        }

        // Snapshot all reflected components.
        if let Some(registry) = resources.get::<ComponentRegistry>() {
            for &type_id in &component_types {
                if let Some(fields) = registry.reflect_get_fields(&type_id, entity) {
                    snapshots.push(ComponentSnapshot { type_id, fields });
                }
            }
        }

        Self {
            entity,
            snapshots,
            component_types,
        }
    }
}

impl EditorCommand for DespawnCommand {
    fn execute(&mut self, resources: &mut Resources) {
        if let Some(alloc) = resources.get_mut::<EntityAllocator>() {
            alloc.despawn(self.entity);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes.unregister_entity(self.entity);
        }
        if let Some(components) = resources.get_mut::<ComponentRegistry>() {
            components.remove_entity(self.entity);
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        // Revive the entity at its original slot.
        let revived = resources
            .get_mut::<EntityAllocator>()
            .is_some_and(|alloc| alloc.revive(self.entity));

        if !revived {
            tracing::warn!(
                "undo: failed to revive entity {} — slot may have been reused",
                self.entity
            );
            return;
        }

        // Re-register into EMPTY archetype first.
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            archetypes
                .register_entity(self.entity, ome_ecs::archetype::ArchetypeId::EMPTY);
        }

        // Restore all components from snapshots.
        for type_id in &self.component_types {
            // Insert default component.
            let mut inserted = false;
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                inserted = registry.insert_default_reflected(type_id, self.entity);
            }
            if inserted {
                // Transition archetype.
                if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                    if let Some(current) = archetypes.entity_archetype(self.entity) {
                        let new_arch =
                            archetypes.archetype_after_add_dynamic(current, *type_id);
                        archetypes.register_entity(self.entity, new_arch);
                    }
                }
            }
        }

        // Restore reflected field values from snapshots.
        for snapshot in &self.snapshots {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                for (field, value) in &snapshot.fields {
                    if let Err(e) = registry.reflect_set_field(
                        &snapshot.type_id,
                        self.entity,
                        field,
                        value.clone(),
                    ) {
                        tracing::warn!("undo: failed to restore field '{field}': {e}");
                    }
                }
            }
        }
    }

    fn description(&self) -> &str {
        "Despawn Entity"
    }
}

// ---------------------------------------------------------------------------
// AddComponentCommand
// ---------------------------------------------------------------------------

pub(crate) struct AddComponentCommand {
    entity: Entity,
    type_id: TypeId,
}

impl AddComponentCommand {
    pub fn new(entity: Entity, type_id: TypeId) -> Self {
        Self { entity, type_id }
    }
}

impl EditorCommand for AddComponentCommand {
    fn execute(&mut self, resources: &mut Resources) {
        let mut inserted = false;
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            inserted = registry.insert_default_reflected(&self.type_id, self.entity);
        }
        if inserted {
            if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                if let Some(current) = archetypes.entity_archetype(self.entity) {
                    let new_arch =
                        archetypes.archetype_after_add_dynamic(current, self.type_id);
                    archetypes.register_entity(self.entity, new_arch);
                }
            }
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.remove_component(self.entity, &self.type_id);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            if let Some(current) = archetypes.entity_archetype(self.entity) {
                let new_arch =
                    archetypes.archetype_after_remove_dynamic(current, self.type_id);
                archetypes.register_entity(self.entity, new_arch);
            }
        }
    }

    fn description(&self) -> &str {
        "Add Component"
    }
}

// ---------------------------------------------------------------------------
// RemoveComponentCommand
// ---------------------------------------------------------------------------

pub(crate) struct RemoveComponentCommand {
    entity: Entity,
    type_id: TypeId,
    /// Snapshot of the removed component's reflected fields.
    snapshot: Option<ComponentSnapshot>,
}

impl RemoveComponentCommand {
    /// Creates the command, snapshotting the component's state before removal.
    pub fn new(resources: &Resources, entity: Entity, type_id: TypeId) -> Self {
        let snapshot = resources
            .get::<ComponentRegistry>()
            .and_then(|reg| reg.reflect_get_fields(&type_id, entity))
            .map(|fields| ComponentSnapshot { type_id, fields });

        Self {
            entity,
            type_id,
            snapshot,
        }
    }
}

impl EditorCommand for RemoveComponentCommand {
    fn execute(&mut self, resources: &mut Resources) {
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.remove_component(self.entity, &self.type_id);
        }
        if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
            if let Some(current) = archetypes.entity_archetype(self.entity) {
                let new_arch =
                    archetypes.archetype_after_remove_dynamic(current, self.type_id);
                archetypes.register_entity(self.entity, new_arch);
            }
        }
    }

    fn undo(&mut self, resources: &mut Resources) {
        // Re-insert the default component.
        let mut inserted = false;
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            inserted = registry.insert_default_reflected(&self.type_id, self.entity);
        }
        if inserted {
            if let Some(archetypes) = resources.get_mut::<ArchetypeRegistry>() {
                if let Some(current) = archetypes.entity_archetype(self.entity) {
                    let new_arch =
                        archetypes.archetype_after_add_dynamic(current, self.type_id);
                    archetypes.register_entity(self.entity, new_arch);
                }
            }
        }

        // Restore field values from snapshot.
        if let Some(ref snapshot) = self.snapshot {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                for (field, value) in &snapshot.fields {
                    if let Err(e) = registry.reflect_set_field(
                        &snapshot.type_id,
                        self.entity,
                        field,
                        value.clone(),
                    ) {
                        tracing::warn!("undo: failed to restore field '{field}': {e}");
                    }
                }
            }
        }
    }

    fn description(&self) -> &str {
        "Remove Component"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ome_ecs::allocator::EntityAllocator;
    use ome_ecs::archetype_registry::ArchetypeRegistry;
    use ome_ecs::component::ComponentRegistry;
    use ome_ecs::query::AccessTracker;
    use ome_ecs::reflect::{FieldKind, FieldMeta, Reflect, ReflectError, ReflectValue};

    // -- Test component -------------------------------------------------------

    #[derive(Debug, Clone, PartialEq)]
    struct Health {
        hp: u32,
        max_hp: u32,
    }

    impl ome_ecs::component::Component for Health {}

    impl Reflect for Health {
        fn reflect_fields(&self) -> &'static [FieldMeta] {
            static FIELDS: &[FieldMeta] = &[
                FieldMeta {
                    name: "hp",
                    type_name: "u32",
                    kind: FieldKind::U32,
                },
                FieldMeta {
                    name: "max_hp",
                    type_name: "u32",
                    kind: FieldKind::U32,
                },
            ];
            FIELDS
        }

        fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
            match field {
                "hp" => Some(ReflectValue::U32(self.hp)),
                "max_hp" => Some(ReflectValue::U32(self.max_hp)),
                _ => None,
            }
        }

        fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
            match field {
                "hp" => match value {
                    ReflectValue::U32(v) => {
                        self.hp = v;
                        Ok(())
                    }
                    other => Err(ReflectError::TypeMismatch {
                        field: "hp".into(),
                        expected: FieldKind::U32,
                        got: other.kind(),
                    }),
                },
                "max_hp" => match value {
                    ReflectValue::U32(v) => {
                        self.max_hp = v;
                        Ok(())
                    }
                    other => Err(ReflectError::TypeMismatch {
                        field: "max_hp".into(),
                        expected: FieldKind::U32,
                        got: other.kind(),
                    }),
                },
                _ => Err(ReflectError::FieldNotFound(field.into())),
            }
        }

        fn reflect_default() -> Self {
            Health {
                hp: 100,
                max_hp: 100,
            }
        }
    }

    // -- Helpers ---------------------------------------------------------------

    fn setup_resources() -> Resources {
        let mut resources = Resources::new();
        resources.insert(EntityAllocator::new());
        resources.insert(ComponentRegistry::new());
        resources.insert(ArchetypeRegistry::new());
        resources.insert(AccessTracker::new());
        resources.insert(ome_ecs::commands::Commands::new());

        // Register Health as reflected.
        resources
            .get_mut::<ComponentRegistry>()
            .unwrap()
            .register_cpu_reflected::<Health>();

        resources
    }

    fn spawn_entity(resources: &mut Resources) -> Entity {
        let mut commands = resources.remove::<ome_ecs::commands::Commands>().unwrap();
        let entity = commands.spawn(resources).id();
        resources.insert(commands);
        entity
    }

    fn add_health(resources: &mut Resources, entity: Entity, hp: u32, max_hp: u32) {
        let type_id = TypeId::of::<Health>();
        resources
            .get_mut::<ComponentRegistry>()
            .unwrap()
            .get_cpu_mut::<Health>()
            .unwrap()
            .insert(entity, Health { hp, max_hp });

        let archetypes = resources.get_mut::<ArchetypeRegistry>().unwrap();
        if let Some(current) = archetypes.entity_archetype(entity) {
            let new_arch = archetypes.archetype_after_add_dynamic(current, type_id);
            archetypes.register_entity(entity, new_arch);
        }
    }

    fn get_hp(resources: &Resources, entity: Entity) -> Option<u32> {
        resources
            .get::<ComponentRegistry>()
            .and_then(|reg| reg.get_cpu::<Health>())
            .and_then(|storage| storage.get(entity))
            .map(|h| h.hp)
    }

    // -- UndoStack tests -------------------------------------------------------

    #[test]
    fn undo_stack_new_is_empty() {
        let stack = UndoStack::new();
        assert!(!stack.can_undo());
        assert!(!stack.can_redo());
    }

    #[test]
    fn set_field_undo_redo() {
        let mut resources = setup_resources();
        let entity = spawn_entity(&mut resources);
        add_health(&mut resources, entity, 50, 100);

        let type_id = TypeId::of::<Health>();
        let cmd = SetFieldCommand::new(
            &resources,
            entity,
            type_id,
            "hp".into(),
            ReflectValue::U32(75),
        )
        .unwrap();

        let mut stack = UndoStack::new();
        stack.execute(Box::new(cmd), &mut resources);
        assert_eq!(get_hp(&resources, entity), Some(75));

        stack.undo(&mut resources);
        assert_eq!(get_hp(&resources, entity), Some(50));

        stack.redo(&mut resources);
        assert_eq!(get_hp(&resources, entity), Some(75));
    }

    #[test]
    fn despawn_undo_restores_entity() {
        let mut resources = setup_resources();
        let entity = spawn_entity(&mut resources);
        add_health(&mut resources, entity, 42, 100);

        let cmd = DespawnCommand::new(&resources, entity);
        let mut stack = UndoStack::new();
        stack.execute(Box::new(cmd), &mut resources);

        // Entity should be dead.
        assert!(!resources
            .get::<EntityAllocator>()
            .unwrap()
            .is_alive(entity));

        stack.undo(&mut resources);

        // Entity should be alive again with the same handle.
        assert!(resources
            .get::<EntityAllocator>()
            .unwrap()
            .is_alive(entity));

        // Health component should be restored.
        assert_eq!(get_hp(&resources, entity), Some(42));
    }

    #[test]
    fn add_component_undo_removes_it() {
        let mut resources = setup_resources();
        let entity = spawn_entity(&mut resources);

        let type_id = TypeId::of::<Health>();
        let cmd = AddComponentCommand::new(entity, type_id);
        let mut stack = UndoStack::new();
        stack.execute(Box::new(cmd), &mut resources);

        // Health should exist with defaults.
        assert_eq!(get_hp(&resources, entity), Some(100));

        stack.undo(&mut resources);

        // Health should be gone.
        assert_eq!(get_hp(&resources, entity), None);
    }

    #[test]
    fn remove_component_undo_restores_it() {
        let mut resources = setup_resources();
        let entity = spawn_entity(&mut resources);
        add_health(&mut resources, entity, 42, 100);

        let type_id = TypeId::of::<Health>();
        let cmd = RemoveComponentCommand::new(&resources, entity, type_id);
        let mut stack = UndoStack::new();
        stack.execute(Box::new(cmd), &mut resources);

        // Health should be gone.
        assert_eq!(get_hp(&resources, entity), None);

        stack.undo(&mut resources);

        // Health should be restored with original values.
        assert_eq!(get_hp(&resources, entity), Some(42));
    }

    #[test]
    fn new_action_clears_redo_stack() {
        let mut resources = setup_resources();
        let entity = spawn_entity(&mut resources);
        add_health(&mut resources, entity, 50, 100);

        let type_id = TypeId::of::<Health>();
        let mut stack = UndoStack::new();

        // Execute and undo.
        let cmd = SetFieldCommand::new(
            &resources,
            entity,
            type_id,
            "hp".into(),
            ReflectValue::U32(75),
        )
        .unwrap();
        stack.execute(Box::new(cmd), &mut resources);
        stack.undo(&mut resources);
        assert!(stack.can_redo());

        // New action should clear redo.
        let cmd2 = SetFieldCommand::new(
            &resources,
            entity,
            type_id,
            "hp".into(),
            ReflectValue::U32(60),
        )
        .unwrap();
        stack.execute(Box::new(cmd2), &mut resources);
        assert!(!stack.can_redo());
    }

    #[test]
    fn despawn_multiple_undo_redo_cycles() {
        let mut resources = setup_resources();
        let entity = spawn_entity(&mut resources);
        add_health(&mut resources, entity, 42, 100);

        let cmd = DespawnCommand::new(&resources, entity);
        let mut stack = UndoStack::new();
        stack.execute(Box::new(cmd), &mut resources);

        // Cycle 1: undo despawn → entity back.
        stack.undo(&mut resources);
        assert!(resources.get::<EntityAllocator>().unwrap().is_alive(entity));
        assert_eq!(get_hp(&resources, entity), Some(42));

        // Cycle 1: redo despawn → entity gone.
        stack.redo(&mut resources);
        assert!(!resources.get::<EntityAllocator>().unwrap().is_alive(entity));
        assert_eq!(get_hp(&resources, entity), None);

        // Cycle 2: undo despawn → entity back again.
        stack.undo(&mut resources);
        assert!(resources.get::<EntityAllocator>().unwrap().is_alive(entity));
        assert_eq!(get_hp(&resources, entity), Some(42));

        // Cycle 2: redo despawn → entity gone again.
        stack.redo(&mut resources);
        assert!(!resources.get::<EntityAllocator>().unwrap().is_alive(entity));

        // Cycle 3: undo one more time.
        stack.undo(&mut resources);
        assert!(resources.get::<EntityAllocator>().unwrap().is_alive(entity));
        assert_eq!(get_hp(&resources, entity), Some(42));
    }

    #[test]
    fn spawn_undo_redo_cycle() {
        let mut resources = setup_resources();
        let mut stack = UndoStack::new();

        // Spawn via command.
        let cmd = SpawnCommand::new();
        stack.execute(Box::new(cmd), &mut resources);

        // Find the spawned entity (should be index 0).
        let entity = {
            let alloc = resources.get::<EntityAllocator>().unwrap();
            assert_eq!(alloc.alive_count(), 1);
            Entity::new(0, 0)
        };
        assert!(resources.get::<EntityAllocator>().unwrap().is_alive(entity));

        // Undo spawn → entity gone.
        stack.undo(&mut resources);
        assert!(!resources.get::<EntityAllocator>().unwrap().is_alive(entity));
        assert_eq!(resources.get::<EntityAllocator>().unwrap().alive_count(), 0);

        // Redo spawn → entity back with same handle.
        stack.redo(&mut resources);
        assert!(resources.get::<EntityAllocator>().unwrap().is_alive(entity));
        assert_eq!(resources.get::<EntityAllocator>().unwrap().alive_count(), 1);
    }

    #[test]
    fn clear_empties_both_stacks() {
        let mut resources = setup_resources();
        let entity = spawn_entity(&mut resources);
        add_health(&mut resources, entity, 50, 100);

        let type_id = TypeId::of::<Health>();
        let mut stack = UndoStack::new();

        let cmd = SetFieldCommand::new(
            &resources,
            entity,
            type_id,
            "hp".into(),
            ReflectValue::U32(75),
        )
        .unwrap();
        stack.execute(Box::new(cmd), &mut resources);
        stack.undo(&mut resources);

        assert!(stack.can_undo() || stack.can_redo());
        stack.clear();
        assert!(!stack.can_undo());
        assert!(!stack.can_redo());
    }
}
