use super::*;

// Shape and size are the point; nothing reads these back.
#[allow(dead_code)]
struct Tag(String);
impl Component for Tag {}

#[test]
fn despawn_cleanup_removes_components() {
    let mut resources = Resources::new();
    let mut alloc = EntityAllocator::with_capacity(4);
    let e = alloc.spawn();

    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<Tag>();

    registry
        .get_cpu_mut::<Tag>()
        .unwrap()
        .insert(e, Tag("test".into()));

    alloc.despawn(e);

    resources.insert(alloc);
    resources.insert(registry);

    component_despawn_cleanup_system(&mut resources);

    let registry = resources.get::<ComponentRegistry>().unwrap();
    assert!(!registry.get_cpu::<Tag>().unwrap().contains(e));
}

#[test]
fn despawn_cleanup_no_panic_without_registry() {
    let mut resources = Resources::new();
    let mut alloc = EntityAllocator::with_capacity(4);
    let e = alloc.spawn();
    alloc.despawn(e);
    resources.insert(alloc);

    // No registry — should not panic.
    component_despawn_cleanup_system(&mut resources);
}

#[test]
fn despawn_cleanup_no_panic_without_allocator() {
    let mut resources = Resources::new();
    // No allocator — should not panic.
    component_despawn_cleanup_system(&mut resources);
}

/// Handing out `&mut` counts as a write, or a panel caching this component stops updating (#1561).
///
/// 🔴 On the registry slot, not on the inbox. The values moved into the columns (#891), so a count
/// kept by a map nobody writes to any more would stop being a signal without failing.
#[test]
fn a_write_moves_the_revision() {
    let mut registry = ComponentRegistry::new();
    let id = registry.register_cpu::<Tag>();
    let entity = crate::Entity::new(1, 0);
    registry
        .get_cpu_mut::<Tag>()
        .unwrap()
        .insert(entity, Tag("a".into()));
    let settled = registry.slot_revision(id);

    assert!(registry.get_cpu::<Tag>().unwrap().get(entity).is_some());
    assert_eq!(registry.slot_revision(id), settled, "a read is not a write");

    registry
        .get_cpu_mut::<Tag>()
        .unwrap()
        .get_mut(entity)
        .expect("present")
        .0 = "b".into();
    assert_ne!(registry.slot_revision(id), settled);
}

/// And the view reports the same number, which is what the editor actually reads.
#[test]
fn the_view_reports_it() {
    let mut registry = ComponentRegistry::new();
    let id = registry.register_cpu::<Tag>();
    registry
        .get_cpu_mut::<Tag>()
        .unwrap()
        .insert(crate::Entity::new(1, 0), Tag("a".into()));

    assert_eq!(
        registry.get_cpu::<Tag>().unwrap().revision(),
        registry.slot_revision(id)
    );
}
