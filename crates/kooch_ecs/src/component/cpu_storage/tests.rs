use super::*;

struct Health(u32);
impl Component for Health {}

fn entity(index: u32, generation: u32) -> Entity {
    Entity::new(index, generation)
}

#[test]
fn insert_and_get() {
    let mut storage = ComponentStorage::<Health>::new();
    let e = entity(0, 0);

    storage.insert(e, Health(100));
    assert_eq!(storage.get(e).unwrap().0, 100);
    assert!(storage.contains(e));
    assert_eq!(storage.len(), 1);
}

#[test]
fn insert_returns_previous() {
    let mut storage = ComponentStorage::<Health>::new();
    let e = entity(0, 0);

    assert!(storage.insert(e, Health(100)).is_none());
    let old = storage.insert(e, Health(200));
    assert_eq!(old.unwrap().0, 100);
    assert_eq!(storage.get(e).unwrap().0, 200);
}

#[test]
fn remove_returns_value() {
    let mut storage = ComponentStorage::<Health>::new();
    let e = entity(0, 0);

    storage.insert(e, Health(50));
    let removed = storage.remove(e);
    assert_eq!(removed.unwrap().0, 50);
    assert!(!storage.contains(e));
    assert!(storage.is_empty());
}

#[test]
fn remove_nonexistent_returns_none() {
    let mut storage = ComponentStorage::<Health>::new();
    assert!(storage.remove(entity(99, 0)).is_none());
}

#[test]
fn get_mut_modifies() {
    let mut storage = ComponentStorage::<Health>::new();
    let e = entity(0, 0);

    storage.insert(e, Health(10));
    storage.get_mut(e).unwrap().0 = 42;
    assert_eq!(storage.get(e).unwrap().0, 42);
}

#[test]
fn iter_all_entries() {
    let mut storage = ComponentStorage::<Health>::new();
    storage.insert(entity(0, 0), Health(1));
    storage.insert(entity(1, 0), Health(2));
    storage.insert(entity(2, 0), Health(3));

    let sum: u32 = storage.iter().map(|(_, h)| h.0).sum();
    assert_eq!(sum, 6);
}

#[test]
fn iter_mut_modifies_all() {
    let mut storage = ComponentStorage::<Health>::new();
    storage.insert(entity(0, 0), Health(1));
    storage.insert(entity(1, 0), Health(2));

    for (_, h) in storage.iter_mut() {
        h.0 *= 10;
    }

    let sum: u32 = storage.iter().map(|(_, h)| h.0).sum();
    assert_eq!(sum, 30);
}

#[test]
fn entity_keyed_by_index_and_generation() {
    let mut storage = ComponentStorage::<Health>::new();
    let e_gen0 = entity(0, 0);
    let e_gen1 = entity(0, 1);

    storage.insert(e_gen0, Health(10));
    // Different generation = different entity key.
    assert!(!storage.contains(e_gen1));
    storage.insert(e_gen1, Health(20));
    assert_eq!(storage.len(), 2);
}

#[test]
fn any_storage_remove_entity() {
    let mut storage = ComponentStorage::<Health>::new();
    let e = entity(0, 0);
    storage.insert(e, Health(100));

    let any_storage: &mut dyn AnyStorage = &mut storage;
    any_storage.remove_entity(e);
    assert!(storage.is_empty());
}
