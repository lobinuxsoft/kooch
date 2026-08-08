use super::*;
use kooch_core::plugin::CorePlugin;
use kooch_ecs::plugin::EcsPlugin;

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugin(CorePlugin);
    app.add_plugin(EcsPlugin);
    app.add_plugin(WorldStreamingPlugin);
    app
}

#[test]
fn plugin_inserts_resources() {
    let app = test_app();
    assert!(app.resources().get::<ChunkManager>().is_some());
    assert!(app.resources().get::<LodRingConfig>().is_some());
}

#[test]
fn system_runs_without_focuses() {
    let mut app = test_app();
    // No focus entities — system runs without panic and produces
    // no chunk activity.
    world_streaming_system(app.resources_mut());
    let m = app.resources().get::<ChunkManager>().unwrap();
    assert_eq!(m.loaded_count(), 0);
}
