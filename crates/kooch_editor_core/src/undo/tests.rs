//! Tests for the undo/redo command pattern. Live in their own file
//! so the parent module stays focused on production code.

use std::any::TypeId;

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::{FieldKind, FieldMeta, Reflect, ReflectError, ReflectValue};

use crate::undo::{
    AddComponentCommand, DespawnCommand, RemoveComponentCommand, SetFieldCommand, SpawnCommand,
    UndoStack,
};

// -- Test component -------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Health {
    hp: u32,
    max_hp: u32,
}

impl kooch_ecs::component::Component for Health {}

impl Reflect for Health {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[
            FieldMeta {
                name: "hp",
                type_name: "u32",
                kind: FieldKind::U32,
                choices: &[],
                bits: &[],
                range: None,
                shown_when: None,
                asset_type: "",
                requires: "",
                doc: "",
                group: "",
            },
            FieldMeta {
                name: "max_hp",
                type_name: "u32",
                kind: FieldKind::U32,
                choices: &[],
                bits: &[],
                range: None,
                shown_when: None,
                asset_type: "",
                requires: "",
                doc: "",
                group: "",
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
    resources.insert(kooch_ecs::commands::Commands::new());

    // Register Health as reflected.
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Health>();

    resources
}

fn spawn_entity(resources: &mut Resources) -> Entity {
    let mut commands = resources.remove::<kooch_ecs::commands::Commands>().unwrap();
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
    assert!(!resources.get::<EntityAllocator>().unwrap().is_alive(entity));

    stack.undo(&mut resources);

    // Entity should be alive again with the same handle.
    assert!(resources.get::<EntityAllocator>().unwrap().is_alive(entity));

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
    let cmd = SpawnCommand::new(vec![], None, crate::actions::SpawnTarget::Active);
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
