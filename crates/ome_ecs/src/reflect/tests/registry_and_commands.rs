use super::common::{Health, Position};
use crate::reflect::ReflectValue;

// -- Registry integration tests ------------------------------------------

#[test]
fn registry_register_cpu_reflected() {
    use crate::component::registry::ComponentRegistry;

    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Health>();

    assert!(registry.has_reflector(&std::any::TypeId::of::<Health>()));
    let metas = registry
        .reflect_field_metas(&std::any::TypeId::of::<Health>())
        .unwrap();
    assert_eq!(metas.len(), 2);
    assert_eq!(metas[0].name, "hp");
}

#[test]
fn registry_reflect_get_fields() {
    use crate::component::registry::ComponentRegistry;
    use crate::entity::Entity;

    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Health>();
    let e = Entity::new(0, 0);
    registry.get_cpu_mut::<Health>().unwrap().insert(
        e,
        Health {
            hp: 42,
            max_hp: 100,
        },
    );

    let fields = registry
        .reflect_get_fields(&std::any::TypeId::of::<Health>(), e)
        .unwrap();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0], ("hp".to_owned(), ReflectValue::U32(42)));
}

#[test]
fn registry_reflect_set_field() {
    use crate::component::registry::ComponentRegistry;
    use crate::entity::Entity;

    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Health>();
    let e = Entity::new(0, 0);
    registry.get_cpu_mut::<Health>().unwrap().insert(
        e,
        Health {
            hp: 50,
            max_hp: 100,
        },
    );

    registry
        .reflect_set_field(
            &std::any::TypeId::of::<Health>(),
            e,
            "hp",
            ReflectValue::U32(75),
        )
        .unwrap();

    let health = registry.get_cpu::<Health>().unwrap().get(e).unwrap();
    assert_eq!(health.hp, 75);
}

#[test]
fn registry_reflected_type_ids() {
    use crate::component::registry::ComponentRegistry;

    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Health>();
    registry.register_cpu::<Position>(); // Not reflected

    let ids = registry.reflected_type_ids();
    assert_eq!(ids.len(), 1);
    assert!(ids.contains(&std::any::TypeId::of::<Health>()));
}

#[test]
fn registry_non_reflected_has_no_reflector() {
    use crate::component::registry::ComponentRegistry;
    use crate::entity::Entity;

    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<Health>(); // Without reflection

    assert!(!registry.has_reflector(&std::any::TypeId::of::<Health>()));
    assert!(
        registry
            .reflect_get_fields(&std::any::TypeId::of::<Health>(), Entity::new(0, 0))
            .is_none()
    );
}

// -- Commands integration tests ------------------------------------------

#[test]
fn commands_insert_reflected() {
    use crate::allocator::EntityAllocator;
    use crate::archetype_registry::ArchetypeRegistry;
    use crate::commands::Commands;
    use crate::component::registry::ComponentRegistry;
    use crate::query::{AccessTracker, Query};
    use ome_core::resource::Resources;

    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());

    let mut commands = Commands::new();
    let entity = commands
        .spawn(&mut resources)
        .insert_reflected(Health {
            hp: 42,
            max_hp: 100,
        })
        .id();
    commands.apply(&mut resources);

    // Verify component works via query.
    let query = Query::<&Health>::new(&resources);
    assert_eq!(query.get(entity).unwrap().hp, 42);

    // Verify reflector was registered.
    let registry = resources.get::<ComponentRegistry>().unwrap();
    assert!(registry.has_reflector(&std::any::TypeId::of::<Health>()));

    // Verify reflected get works.
    let fields = registry
        .reflect_get_fields(&std::any::TypeId::of::<Health>(), entity)
        .unwrap();
    assert_eq!(fields[0], ("hp".to_owned(), ReflectValue::U32(42)));
}
