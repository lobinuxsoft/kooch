use super::*;

#[test]
fn library_names_follow_cargo() {
    let name = library_file_name("my-game");
    assert!(
        name.contains("my_game"),
        "cargo replaces dashes with underscores, got {name}"
    );
    #[cfg(target_os = "linux")]
    assert_eq!(name, "libmy_game.so");
}

#[test]
fn a_project_without_a_library_yields_none() {
    let dir = std::env::temp_dir().join("kooch_no_lib_test");
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(library_path(&dir, "absent"), None);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Opening a project that has no library must not fail, and must not
/// register anything.
#[test]
fn loading_nothing_is_not_an_error() {
    let dir = std::env::temp_dir().join("kooch_no_lib_load_test");
    std::fs::create_dir_all(&dir).unwrap();

    let mut resources = Resources::new();
    assert_eq!(load_project_plugin(&mut resources, &dir, "absent"), 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_source_name_matches_what_the_bridge_derives() {
    assert_eq!(
        source_of(Path::new("/p/target/debug/libmy_game.so")).as_deref(),
        Some("my_game")
    );
    assert_eq!(
        source_of(Path::new("/p/target/debug/my_game.dll")).as_deref(),
        Some("my_game")
    );
}
