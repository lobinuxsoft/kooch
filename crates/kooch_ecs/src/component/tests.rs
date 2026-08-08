use super::*;

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
