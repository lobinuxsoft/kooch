use kooch_core::resource::Resources;
use kooch_core::window_mode::WindowMode;

/// 🔴 Absent means no opinion: a default inserted here would take the display on the first frame of
/// every game without settings.
#[test]
fn no_mode_means_no_change() {
    let mut resources = Resources::new();
    super::apply_window_mode_system(&mut resources);
    assert!(resources.get::<WindowMode>().is_none());
}

/// Without a window the resource stays absent — an empty list would tell an options menu there are
/// no resolutions.
#[test]
fn no_window_publishes_nothing() {
    let mut resources = Resources::new();
    super::publish_display_modes_system(&mut resources);
    assert!(
        resources
            .get::<kooch_core::window_mode::DisplayModes>()
            .is_none()
    );
}

/// 🔴 The editor loads a project's `.rendersettings` too, so a fullscreen `window_mode` would take
/// the editor full screen.
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
            .filter(|name| name.contains("window_mode") || name.contains("display_modes"))
            .count()
    };

    assert_eq!(named(true), 2, "a game's window follows and enumerates");
    assert_eq!(named(false), 0, "a tool's window does neither");
}

/// The default is a game's window: a project that adds the plugin
/// without an opinion gets the setting it authored.
#[test]
fn the_default_follows_the_setting() {
    assert!(crate::WindowPlugin::default().applies_window_mode);
}
