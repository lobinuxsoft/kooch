use super::*;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::component::DynamicField;

fn world() -> (Resources, Entity) {
    let mut resources = Resources::new();
    let mut alloc = EntityAllocator::new();
    let entity = alloc.spawn();
    resources.insert(alloc);
    resources.insert(DynamicComponents::new());

    let mut types = DynamicTypeRegistry::new();
    types
        .register(DynamicType {
            type_name: "my_game::Health".into(),
            fields: vec![
                DynamicField {
                    name: "current".into(),
                    kind: FieldKind::U32,
                },
                DynamicField {
                    name: "regen".into(),
                    kind: FieldKind::F32,
                },
            ],
            defaults: Vec::new(),
            source: "my_game".into(),
        })
        .unwrap();
    resources.insert(types);
    (resources, entity)
}

fn fields_of(resources: &Resources, entity: Entity) -> Vec<(String, ReflectValue)> {
    resources
        .get::<DynamicComponents>()
        .unwrap()
        .iter_entity(entity)
        .find(|(name, _)| *name == "my_game::Health")
        .map(|(_, f)| f.to_vec())
        .unwrap_or_default()
}

#[test]
fn adding_gives_every_field_a_starting_value() {
    let (mut resources, entity) = world();
    let mut cmd = AddDynamicComponentCommand::new(&resources, entity, "my_game::Health")
        .expect("type is registered");

    cmd.execute(&mut resources);

    let fields = fields_of(&resources, entity);
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0], ("current".into(), ReflectValue::U32(0)));
    assert_eq!(fields[1], ("regen".into(), ReflectValue::F32(0.0)));
}

#[test]
fn undo_removes_what_add_created() {
    let (mut resources, entity) = world();
    let mut cmd = AddDynamicComponentCommand::new(&resources, entity, "my_game::Health").unwrap();

    cmd.execute(&mut resources);
    cmd.undo(&mut resources);

    assert!(fields_of(&resources, entity).is_empty());
}

/// A menu entry left over from a plugin that has been unloaded must
/// not produce a command that writes a component with no schema.
#[test]
fn an_unregistered_type_yields_no_command() {
    let (resources, entity) = world();
    assert!(AddDynamicComponentCommand::new(&resources, entity, "gone::Type").is_none());
}

#[test]
fn editing_a_field_round_trips_through_undo() {
    let (mut resources, entity) = world();
    AddDynamicComponentCommand::new(&resources, entity, "my_game::Health")
        .unwrap()
        .execute(&mut resources);

    let mut cmd = SetDynamicFieldCommand::new(
        &resources,
        entity,
        "my_game::Health",
        "current".into(),
        ReflectValue::U32(75),
    )
    .expect("component and field exist");

    cmd.execute(&mut resources);
    assert_eq!(fields_of(&resources, entity)[0].1, ReflectValue::U32(75));

    cmd.undo(&mut resources);
    assert_eq!(fields_of(&resources, entity)[0].1, ReflectValue::U32(0));
}

#[test]
fn editing_an_absent_field_yields_no_command() {
    let (mut resources, entity) = world();
    AddDynamicComponentCommand::new(&resources, entity, "my_game::Health")
        .unwrap()
        .execute(&mut resources);

    assert!(
        SetDynamicFieldCommand::new(
            &resources,
            entity,
            "my_game::Health",
            "nonexistent".into(),
            ReflectValue::U32(1),
        )
        .is_none()
    );
    let _ = &mut resources;
}

/// Removing keeps the authored values, so undo restores the
/// component rather than a blank one.
#[test]
fn removing_and_undoing_restores_the_values() {
    let (mut resources, entity) = world();
    AddDynamicComponentCommand::new(&resources, entity, "my_game::Health")
        .unwrap()
        .execute(&mut resources);
    SetDynamicFieldCommand::new(
        &resources,
        entity,
        "my_game::Health",
        "current".into(),
        ReflectValue::U32(42),
    )
    .unwrap()
    .execute(&mut resources);

    let mut cmd = RemoveDynamicComponentCommand::new(&resources, entity, "my_game::Health");
    cmd.execute(&mut resources);
    assert!(fields_of(&resources, entity).is_empty());

    cmd.undo(&mut resources);
    assert_eq!(
        fields_of(&resources, entity)[0].1,
        ReflectValue::U32(42),
        "undo restored a blank component instead of the authored one"
    );
}

/// Redo after a plugin reload must use the type's current shape.
#[test]
fn redo_reads_the_schema_again() {
    let (mut resources, entity) = world();
    let mut cmd = AddDynamicComponentCommand::new(&resources, entity, "my_game::Health").unwrap();
    cmd.execute(&mut resources);
    cmd.undo(&mut resources);

    // The plugin was rebuilt with an extra field.
    let mut types = resources.remove::<DynamicTypeRegistry>().unwrap();
    types
        .register(DynamicType {
            type_name: "my_game::Health".into(),
            fields: vec![DynamicField {
                name: "shield".into(),
                kind: FieldKind::Bool,
            }],
            defaults: Vec::new(),
            source: "my_game".into(),
        })
        .unwrap();
    resources.insert(types);

    cmd.execute(&mut resources);

    let fields = fields_of(&resources, entity);
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0], ("shield".into(), ReflectValue::Bool(false)));
}
