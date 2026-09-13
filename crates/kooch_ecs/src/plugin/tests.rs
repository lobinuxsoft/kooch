use super::*;

#[test]
fn plugin_registers_allocator_and_registry() {
    let mut app = App::new();
    app.add_plugin(EcsPlugin);

    assert!(app.resources().get::<EntityAllocator>().is_some());
    assert!(app.resources().get::<ComponentRegistry>().is_some());
    assert!(app.resources().get::<AccessTracker>().is_some());
}

/// `Spin` is registered by the engine, not by a feature.
#[test]
fn a_spin_ships_unconditionally() {
    let mut app = App::new();
    app.add_plugin(EcsPlugin);
    super::register_builtin_components(app.resources_mut());

    let registry = app
        .resources()
        .get::<ComponentRegistry>()
        .expect("EcsPlugin inserts the registry");
    assert!(
        registry.has_reflector(&std::any::TypeId::of::<crate::testing::spin::Spin>()),
        "Spin is not registered; an exported scene loses every pivot silently"
    );
    assert_eq!(
        std::any::type_name::<crate::testing::spin::Spin>(),
        "kooch_ecs::testing::spin::Spin",
        "the serialised type name moved; every scene holding a Spin drops it on load"
    );
}
