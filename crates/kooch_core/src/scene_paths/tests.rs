use super::*;

/// The default path and the extension were two literals in two crates,
/// and a rename moved one. They are one fact now, and this fails if
/// they ever drift apart again.
#[test]
fn the_default_scene_path_carries_the_scene_extension() {
    assert!(
        DEFAULT_SCENE_REL_PATH.ends_with(&format!(".{SCENE_EXTENSION}")),
        "{DEFAULT_SCENE_REL_PATH} does not end in .{SCENE_EXTENSION}",
    );
}

/// The whole point of two extensions is that they differ.
#[test]
fn a_scene_and_a_prefab_are_told_apart() {
    assert_ne!(SCENE_EXTENSION, PREFAB_EXTENSION);
}

/// The manifest is a file name, not an extension: it carries its dot
/// in the middle. Asserting it stops a future rename from turning it
/// into a bare `kooch` that `join()` would happily create as a folder.
#[test]
fn the_manifest_is_a_file_name() {
    assert!(PROJECT_MANIFEST_FILE.contains('.'));
    assert!(!PROJECT_MANIFEST_FILE.starts_with('.'));
    assert!(!PROJECT_MANIFEST_FILE.ends_with('.'));
}

/// A leading dot would make every `format!(".{ext}")` produce `..scene`.
#[test]
fn an_extension_is_bare() {
    for ext in [SCENE_EXTENSION, PREFAB_EXTENSION] {
        assert!(!ext.starts_with('.'), "{ext} should not carry its dot");
        assert!(!ext.is_empty());
    }
}
