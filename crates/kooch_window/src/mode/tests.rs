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
