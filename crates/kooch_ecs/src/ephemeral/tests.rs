use super::*;

struct MarkerA;
struct MarkerB;
struct Other;

#[test]
fn empty_registry_does_not_match() {
    let registry = EphemeralComponents::new();
    assert!(!registry.contains(&TypeId::of::<MarkerA>()));
    assert!(!registry.intersects([&TypeId::of::<MarkerA>(), &TypeId::of::<Other>()]));
}

#[test]
fn insert_and_contains() {
    let mut registry = EphemeralComponents::new();
    registry.insert(TypeId::of::<MarkerA>());
    assert!(registry.contains(&TypeId::of::<MarkerA>()));
    assert!(!registry.contains(&TypeId::of::<MarkerB>()));
}

#[test]
fn intersects_detects_any_match() {
    let mut registry = EphemeralComponents::new();
    registry.insert(TypeId::of::<MarkerA>());

    let with_marker = [TypeId::of::<MarkerA>(), TypeId::of::<Other>()];
    let without_marker = [TypeId::of::<MarkerB>(), TypeId::of::<Other>()];

    assert!(registry.intersects(with_marker.iter()));
    assert!(!registry.intersects(without_marker.iter()));
}

#[test]
fn insert_is_idempotent() {
    let mut registry = EphemeralComponents::new();
    registry.insert(TypeId::of::<MarkerA>());
    registry.insert(TypeId::of::<MarkerA>());
    assert_eq!(registry.types().len(), 1);
}
