use super::*;
use std::collections::HashSet;

#[test]
fn creation() {
    let e = Entity::new(0, 1);
    assert_eq!(e.index(), 0);
    assert_eq!(e.generation(), 1);
}

#[test]
fn invalid_sentinel() {
    assert!(!Entity::INVALID.is_valid());
    assert_eq!(Entity::INVALID.index(), u32::MAX);
    assert_eq!(Entity::INVALID.generation(), 0);
}

#[test]
fn valid_entity() {
    assert!(Entity::new(0, 0).is_valid());
    assert!(Entity::new(42, 7).is_valid());
}

#[test]
fn equality() {
    let a = Entity::new(1, 2);
    let b = Entity::new(1, 2);
    let c = Entity::new(1, 3);
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn to_gpu() {
    let e = Entity::new(42, 5);
    assert_eq!(e.to_gpu(), 42);
}

#[test]
fn hash_works_in_set() {
    let mut set = HashSet::new();
    set.insert(Entity::new(0, 0));
    set.insert(Entity::new(0, 0));
    set.insert(Entity::new(1, 0));
    assert_eq!(set.len(), 2);
}

#[test]
fn display() {
    let e = Entity::new(3, 7);
    assert_eq!(format!("{e}"), "Entity(index=3, gen=7)");
}
