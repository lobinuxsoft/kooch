use std::path::Path;

use super::EditorConfig;

/// 🔴 Per project, because a line that silently applied to the other
/// one too is how a capture measures something nobody asked for.
#[test]
fn a_line_belongs_to_one_project() {
    let mut config = EditorConfig::default();
    config.set_launch_env(Path::new("/a"), "KOOCH_SHADING_PAD=4".to_owned());
    assert_eq!(
        config.launch_env_for(Path::new("/a")),
        "KOOCH_SHADING_PAD=4"
    );
    assert_eq!(config.launch_env_for(Path::new("/b")), "");
}

/// Setting it twice replaces rather than appends, or the second
/// launch would still be reading the first line.
#[test]
fn setting_it_again_replaces() {
    let mut config = EditorConfig::default();
    config.set_launch_env(Path::new("/a"), "KOOCH_SHADING_PAD=4".to_owned());
    config.set_launch_env(Path::new("/a"), "KOOCH_SHADING_PAD=200".to_owned());
    assert_eq!(config.launch_env.len(), 1);
    assert_eq!(
        config.launch_env_for(Path::new("/a")),
        "KOOCH_SHADING_PAD=200"
    );
}

/// Clearing the field leaves no entry behind — a stored blank is a
/// line somebody later reads as a setting they made.
#[test]
fn an_empty_line_clears_it() {
    let mut config = EditorConfig::default();
    config.set_launch_env(Path::new("/a"), "KOOCH_SHADING_PAD=4".to_owned());
    config.set_launch_env(Path::new("/a"), "   ".to_owned());
    assert!(config.launch_env.is_empty());
}

/// A config written before this field existed has to load, the same
/// as any other asset on any other disk.
#[test]
fn an_older_config_still_loads() {
    let config: EditorConfig =
        ron::from_str("(recent_projects: [])").expect("a config without the field");
    assert!(config.launch_env.is_empty());
}
