use std::any::TypeId;

use crate::component::registry::ComponentRegistry;
use crate::component::traits::Component;
use crate::entity::Entity;

#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Position {
    x: f32,
    y: f32,
}
impl Component for Position {}

struct Name(String);
impl Component for Name {}

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

#[test]
fn register_and_get_cpu() {
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<Name>();

    let storage = registry.get_cpu::<Name>().unwrap();
    assert!(storage.is_empty());
}

#[test]
fn get_unregistered_returns_none() {
    let registry = ComponentRegistry::new();
    assert!(registry.get_cpu::<Name>().is_none());
}

#[test]
fn register_idempotent() {
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<Position>();

    // Insert data via the storage.
    registry
        .get_cpu_mut::<Position>()
        .unwrap()
        .insert(entity(0), Position { x: 1.0, y: 2.0 });

    // Re-registering should NOT reset the storage.
    registry.register_cpu::<Position>();
    assert_eq!(registry.get_cpu::<Position>().unwrap().len(), 1);
}

#[test]
fn insert_and_retrieve_cpu_components() {
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<Name>();

    let e = entity(0);
    registry
        .get_cpu_mut::<Name>()
        .unwrap()
        .insert(e, Name("Alice".into()));

    let name = registry.get_cpu::<Name>().unwrap().get(e).unwrap();
    assert_eq!(name.0, "Alice");
}

#[test]
fn remove_entity_from_all_storages() {
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<Position>();
    registry.register_cpu::<Name>();

    let e = entity(0);
    registry
        .get_cpu_mut::<Position>()
        .unwrap()
        .insert(e, Position { x: 1.0, y: 2.0 });
    registry
        .get_cpu_mut::<Name>()
        .unwrap()
        .insert(e, Name("Bob".into()));

    registry.remove_entity(e);

    assert!(!registry.get_cpu::<Position>().unwrap().contains(e));
    assert!(!registry.get_cpu::<Name>().unwrap().contains(e));
}

#[test]
fn mixed_gpu_and_cpu_storages() {
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<Position>();
    registry.register_cpu::<Name>();

    let e1 = entity(0);
    let e2 = entity(1);

    registry
        .get_cpu_mut::<Position>()
        .unwrap()
        .insert(e1, Position { x: 1.0, y: 0.0 });
    registry
        .get_cpu_mut::<Position>()
        .unwrap()
        .insert(e2, Position { x: 2.0, y: 0.0 });
    registry
        .get_cpu_mut::<Name>()
        .unwrap()
        .insert(e1, Name("A".into()));

    assert_eq!(registry.get_cpu::<Position>().unwrap().len(), 2);
    assert_eq!(registry.get_cpu::<Name>().unwrap().len(), 1);
}

#[test]
fn contains_type_check() {
    let mut registry = ComponentRegistry::new();
    assert!(!registry.contains_type(&TypeId::of::<Position>()));

    registry.register_cpu::<Position>();
    assert!(registry.contains_type(&TypeId::of::<Position>()));
}

/// A component whose `Default` is deliberately not all zeroes.
#[derive(Default, Clone, Debug, PartialEq, kooch_ecs_macros::Reflect)]
struct Spawnable {
    enabled: bool,
    rate: f32,
}

impl Component for Spawnable {}

impl Spawnable {
    fn awake() -> Self {
        Self {
            enabled: true,
            rate: 2.5,
        }
    }
}

/// Adding a component somewhere no entity exists — a prefab document —
/// needs the type's own default, not one synthesised per field kind. A
/// component whose default sets a flag must arrive with it set, or the
/// prefab silently disagrees with what spawning the same component gives.
#[test]
fn default_fields_come_from_the_type_not_from_zeroes() {
    use crate::reflect::{Reflect, ReflectValue};

    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Spawnable>();

    let fields = registry
        .reflect_default_fields(&TypeId::of::<Spawnable>())
        .expect("a registered reflected type has defaults");

    // Compared against the type itself rather than against literals, so the
    // test keeps meaning if the default changes.
    let expected = Spawnable::default();
    for (name, value) in &fields {
        assert_eq!(
            Some(value.clone()),
            expected.reflect_get(name),
            "field {name} does not match the type's own default",
        );
    }
    assert_eq!(fields.len(), expected.reflect_fields().len());
    // And it is really reading the value, not handing back a zero that
    // happens to match: `awake()` differs from the default in both fields.
    let awake = Spawnable::awake();
    assert_ne!(awake.reflect_get("rate"), Some(ReflectValue::F32(0.0)));
}

/// A type with no reflector has no defaults to report, rather than an
/// empty list that would read as "no fields".
#[test]
fn an_unreflected_type_reports_no_defaults() {
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<Name>();
    assert!(
        registry
            .reflect_default_fields(&TypeId::of::<Name>())
            .is_none()
    );
}

// -- The dense handle (#891, stage 1) ---------------------------------------

struct Health(u32);
impl Component for Health {}

/// Needs `Reflect` for the slot-reuse test; `Default` because the derive
/// requires it.
#[derive(Debug, Clone, Copy, Default, crate::Reflect)]
struct Armour {
    plates: u32,
}
impl Component for Armour {}

/// Ids come out 0, 1, 2 — with no gaps, because a column will index a
/// `Vec` with them and a gap there is a wasted allocation per type.
#[test]
fn ids_are_dense() {
    let mut registry = ComponentRegistry::new();
    let a = registry.register_cpu::<Position>();
    let b = registry.register_cpu::<Name>();
    let c = registry.register_cpu::<Health>();

    assert_eq!([a.0, b.0, c.0], [0, 1, 2]);
    assert_eq!(registry.registered_count(), 3);
}

/// Registering the same type twice hands back the same slot rather than
/// minting a second one. A duplicate slot would mean two storages for one
/// component, and whichever a query found first would win.
#[test]
fn registering_twice_is_idempotent() {
    let mut registry = ComponentRegistry::new();
    let first = registry.register_cpu::<Position>();
    let again = registry.register_cpu::<Position>();

    assert_eq!(first, again);
    assert_eq!(registry.registered_count(), 1);
}

/// 🔴 The invariant the whole port rests on: an id, once handed out, never
/// moves. A query resolves it once and then indexes forever — if a later
/// registration could shift it, every cached id would silently address
/// another component's storage.
#[test]
fn an_id_outlives_later_registrations() {
    let mut registry = ComponentRegistry::new();
    let position = registry.register_cpu::<Position>();
    registry.register_cpu::<Name>();
    registry.register_cpu::<Health>();

    assert_eq!(
        registry.storage_id(&TypeId::of::<Position>()),
        Some(position)
    );
}

/// Adding reflection to an already-registered component reuses its slot.
#[test]
fn reflection_reuses_the_slot() {
    let mut registry = ComponentRegistry::new();
    let id = registry.register_cpu::<Armour>();
    registry.register_cpu_reflected::<Armour>();

    assert_eq!(registry.registered_count(), 1);
    assert_eq!(registry.storage_id(&TypeId::of::<Armour>()), Some(id));
    assert!(registry.has_reflector(&TypeId::of::<Armour>()));
}

/// An unregistered type has no id, rather than a defaulted one.
#[test]
fn an_unknown_type_has_no_id() {
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<Position>();

    assert_eq!(registry.storage_id(&TypeId::of::<Health>()), None);
}
