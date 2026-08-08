use super::*;
use std::collections::BTreeSet;

struct Position;
struct Velocity;
struct Health;

fn archetype_with<T: 'static>() -> Archetype {
    let mut components = BTreeSet::new();
    components.insert(TypeId::of::<T>());
    Archetype::new(components)
}

fn archetype_with_2<A: 'static, B: 'static>() -> Archetype {
    let mut components = BTreeSet::new();
    components.insert(TypeId::of::<A>());
    components.insert(TypeId::of::<B>());
    Archetype::new(components)
}

#[test]
fn unit_filter_matches_all() {
    let arch = archetype_with::<Position>();
    assert!(<() as QueryFilter>::matches_archetype(&arch));
}

#[test]
fn with_filter() {
    let arch = archetype_with::<Position>();
    assert!(With::<Position>::matches_archetype(&arch));
    assert!(!With::<Velocity>::matches_archetype(&arch));
}

#[test]
fn without_filter() {
    let arch = archetype_with::<Position>();
    assert!(Without::<Velocity>::matches_archetype(&arch));
    assert!(!Without::<Position>::matches_archetype(&arch));
}

#[test]
fn combined_filter() {
    let arch = archetype_with_2::<Position, Velocity>();
    assert!(<(With<Position>, Without<Health>)>::matches_archetype(
        &arch
    ));
    assert!(!<(With<Position>, Without<Velocity>)>::matches_archetype(
        &arch
    ));
}
