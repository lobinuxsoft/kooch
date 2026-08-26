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

/// 🔴 The field the runtime ignored until #808. A manifest that names a
/// scene has to be readable by a game, which does not link the editor
/// and therefore cannot use `ProjectManifest`.
#[test]
fn a_manifest_names_its_main_scene() {
    let manifest = r#"(
    name: "roll-a-ball",
    version: "0.1.0",
    engine_version: "0.2.0",
    main_scene: Some("assets/scenes/many_lights.scene"),
    window: (
        title: "roll-a-ball",
        width: 1280,
        height: 720,
    ),
)"#;
    assert_eq!(
        main_scene_of(manifest).as_deref(),
        Some("assets/scenes/many_lights.scene"),
    );
}

/// The manifest grows; this function must not have to.
#[test]
fn unknown_fields_are_ignored() {
    let manifest = r#"(
    name: "x",
    main_scene: Some("assets/scenes/a.scene"),
    something_added_next_year: 3,
)"#;
    assert_eq!(main_scene_of(manifest).is_some(), true);
}

#[test]
fn a_manifest_without_one_names_nothing() {
    assert_eq!(main_scene_of(r#"(name: "x", main_scene: None)"#), None);
    // An empty string is a field somebody cleared, not a path.
    assert_eq!(main_scene_of(r#"(main_scene: Some(""))"#), None);
    // And nonsense is not a scene either.
    assert_eq!(main_scene_of("not a manifest"), None);
}

/// ⚠️ Both forms look plausible and only one resolves. `roll-a-ball`
/// carried the short one, which joined to a path that did not exist —
/// and the guard around the load skipped it in silence.
#[test]
fn the_short_form_is_normalised() {
    assert_eq!(
        normalise_main_scene("scenes/default.scene"),
        "assets/scenes/default.scene",
    );
    assert_eq!(
        normalise_main_scene("./scenes/default.scene"),
        "assets/scenes/default.scene",
    );
    // Already right, left alone.
    assert_eq!(
        normalise_main_scene(DEFAULT_SCENE_REL_PATH),
        DEFAULT_SCENE_REL_PATH,
    );
    // Somebody's own layout is their business.
    assert_eq!(normalise_main_scene("levels/one.scene"), "levels/one.scene");
}
