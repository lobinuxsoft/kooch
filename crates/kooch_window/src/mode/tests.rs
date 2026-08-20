use kooch_core::resource::Resources;
use kooch_core::window_mode::WindowMode;

/// 🔴 Absent means "no opinion" and the system must not invent one.
/// A game with no settings asset, and a test that made its own
/// window, keep the window they have — the rule the whole quality
/// bundle is built on. A default inserted here would take the
/// display on the first frame of every such game.
#[test]
fn no_mode_means_no_change() {
    let mut resources = Resources::new();
    super::apply_window_mode_system(&mut resources);
    assert!(resources.get::<WindowMode>().is_none());
}

/// 🔴 The editor adds this plugin AND the asset plugin that publishes a
/// project's `.rendersettings`, so a project whose `window_mode` is
/// fullscreen would take the editor full screen. Registered or not,
/// rather than registered and skipped: there is nothing per-frame to
/// decide.
#[test]
fn a_tool_window_never_registers_it() {
    use kooch_core::app::App;
    use kooch_core::plugin::Plugin;
    use kooch_core::stage::Stage;

    let named = |applies: bool| {
        let mut app = App::new();
        crate::WindowPlugin {
            title: "t".to_owned(),
            width: 1,
            height: 1,
            applies_window_mode: applies,
        }
        .build(&mut app);
        app.schedule()
            .system_names(Stage::Last)
            .iter()
            .any(|name| name.contains("apply_window_mode"))
    };

    assert!(named(true), "a game's window has to follow the setting");
    assert!(!named(false), "a tool's window must not");
}

/// The default is a game's window: a project that adds the plugin
/// without an opinion gets the setting it authored.
#[test]
fn the_default_follows_the_setting() {
    assert!(crate::WindowPlugin::default().applies_window_mode);
}
