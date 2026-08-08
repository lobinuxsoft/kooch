use super::*;
use kooch_core::event::Events;
use kooch_core::plugin::MinimalPlugins;

#[test]
fn window_config_default() {
    let config = WindowConfig::default();
    assert_eq!(config.title, "Kóoch");
    assert_eq!(config.width, 1280);
    assert_eq!(config.height, 720);
}

#[test]
fn window_plugin_registers_events() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin::default());

    assert!(app.resources().contains::<WindowConfig>());
    assert!(app.resources().contains::<Events<WindowResized>>());
    assert!(app.resources().contains::<Events<WindowCloseRequested>>());
}

#[test]
fn window_plugin_custom_config() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin {
        title: "Test Game".to_string(),
        width: 1920,
        height: 1080,
    });

    let config = app.resources().get::<WindowConfig>().unwrap();
    assert_eq!(config.title, "Test Game");
    assert_eq!(config.width, 1920);
    assert_eq!(config.height, 1080);
}
