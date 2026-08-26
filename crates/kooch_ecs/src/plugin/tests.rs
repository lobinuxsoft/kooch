use super::*;

#[test]
fn plugin_registers_allocator_and_registry() {
    let mut app = App::new();
    app.add_plugin(EcsPlugin);

    assert!(app.resources().get::<EntityAllocator>().is_some());
    assert!(app.resources().get::<ComponentRegistry>().is_some());
    assert!(app.resources().get::<AccessTracker>().is_some());
}
