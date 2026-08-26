use super::*;

struct Position;
struct Velocity;
struct Health;

#[test]
fn empty_archetype_id() {
    let id = ArchetypeId::from_components(&BTreeSet::new());
    assert_eq!(id, ArchetypeId::EMPTY);
}

#[test]
fn deterministic_id() {
    let mut set_a = BTreeSet::new();
    set_a.insert(TypeId::of::<Position>());
    set_a.insert(TypeId::of::<Velocity>());

    let mut set_b = BTreeSet::new();
    set_b.insert(TypeId::of::<Velocity>());
    set_b.insert(TypeId::of::<Position>());

    assert_eq!(
        ArchetypeId::from_components(&set_a),
        ArchetypeId::from_components(&set_b),
    );
}

#[test]
fn different_components_different_id() {
    let mut set_a = BTreeSet::new();
    set_a.insert(TypeId::of::<Position>());

    let mut set_b = BTreeSet::new();
    set_b.insert(TypeId::of::<Velocity>());

    assert_ne!(
        ArchetypeId::from_components(&set_a),
        ArchetypeId::from_components(&set_b),
    );
}

#[test]
fn non_empty_differs_from_empty() {
    let mut set = BTreeSet::new();
    set.insert(TypeId::of::<Position>());

    assert_ne!(ArchetypeId::from_components(&set), ArchetypeId::EMPTY);
}

#[test]
fn archetype_new() {
    let mut components = BTreeSet::new();
    components.insert(TypeId::of::<Position>());
    components.insert(TypeId::of::<Velocity>());

    let arch = Archetype::new(components.clone());
    assert_eq!(arch.id(), ArchetypeId::from_components(&components));
    assert_eq!(arch.components(), &components);
    assert!(arch.is_empty());
    assert_eq!(arch.len(), 0);
}

#[test]
fn has_component() {
    let mut components = BTreeSet::new();
    components.insert(TypeId::of::<Position>());

    let arch = Archetype::new(components);
    assert!(arch.has_component::<Position>());
    assert!(!arch.has_component::<Velocity>());
}

#[test]
fn add_and_remove_entity() {
    let mut arch = Archetype::new(BTreeSet::new());
    let e1 = Entity::new(0, 0);
    let e2 = Entity::new(1, 0);

    arch.add_entity(e1);
    arch.add_entity(e2);
    assert_eq!(arch.len(), 2);
    assert_eq!(arch.entities(), &[e1, e2]);

    assert!(arch.remove_entity(e1));
    assert_eq!(arch.len(), 1);
    assert_eq!(arch.entities(), &[e2]);

    assert!(!arch.remove_entity(e1));
}

#[test]
fn swap_remove_preserves_density() {
    let mut arch = Archetype::new(BTreeSet::new());
    let e1 = Entity::new(0, 0);
    let e2 = Entity::new(1, 0);
    let e3 = Entity::new(2, 0);

    arch.add_entity(e1);
    arch.add_entity(e2);
    arch.add_entity(e3);

    // Removing e1 should swap e3 into position 0.
    arch.remove_entity(e1);
    assert_eq!(arch.len(), 2);
    assert!(arch.entities().contains(&e2));
    assert!(arch.entities().contains(&e3));
}
