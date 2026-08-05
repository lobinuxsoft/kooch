use super::common::{Health, Position};
use crate::reflect::{Reflect, ReflectAccessor, ReflectError, ReflectValue, TypedReflectAccessor};

// -- TypedReflectAccessor tests ------------------------------------------

#[test]
fn accessor_fields_returns_metadata() {
    let accessor = TypedReflectAccessor::<Health>::new_cpu();
    let fields = accessor.fields();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "hp");
}

#[test]
fn accessor_get_fields_from_cpu_storage() {
    use crate::component::cpu_storage::ComponentStorage;
    use crate::entity::Entity;

    let mut storage = ComponentStorage::<Health>::new();
    let e = Entity::new(0, 0);
    storage.insert(
        e,
        Health {
            hp: 42,
            max_hp: 100,
        },
    );

    let accessor = TypedReflectAccessor::<Health>::new_cpu();
    let fields = accessor.get_fields(&storage, e).unwrap();

    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0], ("hp".to_owned(), ReflectValue::U32(42)));
    assert_eq!(fields[1], ("max_hp".to_owned(), ReflectValue::U32(100)));
}

#[test]
fn accessor_get_fields_missing_entity() {
    use crate::component::cpu_storage::ComponentStorage;
    use crate::entity::Entity;

    let storage = ComponentStorage::<Health>::new();
    let e = Entity::new(99, 0);

    let accessor = TypedReflectAccessor::<Health>::new_cpu();
    assert!(accessor.get_fields(&storage, e).is_none());
}

#[test]
fn accessor_set_field_on_cpu_storage() {
    use crate::component::cpu_storage::ComponentStorage;
    use crate::entity::Entity;

    let mut storage = ComponentStorage::<Health>::new();
    let e = Entity::new(0, 0);
    storage.insert(
        e,
        Health {
            hp: 50,
            max_hp: 100,
        },
    );

    let accessor = TypedReflectAccessor::<Health>::new_cpu();
    accessor
        .set_field(&mut storage, e, "hp", ReflectValue::U32(75))
        .unwrap();

    assert_eq!(storage.get(e).unwrap().hp, 75);
}

#[test]
fn accessor_default_value() {
    let accessor = TypedReflectAccessor::<Health>::new_cpu();
    let boxed = accessor.default_value();
    let health = boxed.downcast::<Health>().unwrap();
    assert_eq!(health.hp, 100);
    assert_eq!(health.max_hp, 100);
}
