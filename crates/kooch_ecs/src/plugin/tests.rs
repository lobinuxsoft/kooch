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
///
/// # 🔴 What an unregistered component does is the whole point
///
/// It is DROPPED on load rather than refused. So a scene authored with
/// orbiting lights, exported by a build that did not carry the
/// component, opens with every pivot gone — lights that move in the
/// editor and stand still in the game, no error and nothing in the log.
/// That shipped.
///
/// The name asserted here is the SERIALISED one. `kooch_ecs::testing`
/// keeps its misleading name precisely because scenes resolve components
/// by this string; see that module for why renaming it is a data
/// migration and not a refactor.
#[test]
fn a_spin_survives_a_build_without_features() {
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
