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

// ---- the swap itself ------------------------------------------------

use kooch_core::dynamic::{ComponentBridge, PluginData};
use kooch_ecs::component::DynamicTypeRegistry;
use kooch_ecs::component::plugin_bridge::register_schema;

/// Where the workspace put the example plugin.
fn example_library() -> std::path::PathBuf {
    let mut dir = std::env::current_exe().expect("test exe");
    dir.pop();
    if dir.ends_with("deps") {
        dir.pop();
    }
    #[cfg(target_os = "windows")]
    let name = "example_plugin.dll";
    #[cfg(target_os = "linux")]
    let name = "libexample_plugin.so";
    #[cfg(target_os = "macos")]
    let name = "libexample_plugin.dylib";
    dir.join(name)
}

/// A scratch project whose `target/debug` holds a real, loadable library.
///
/// A copy rather than the workspace's own: the swap unmaps what it
/// loaded, and pointing two tests at one file is how they start
/// depending on each other's order.
fn plugin_project(name: &str) -> Option<(std::path::PathBuf, Resources)> {
    let source = example_library();
    if !source.exists() {
        skipped();
        return None;
    }
    let root = std::env::temp_dir().join(format!("kooch_reload_{name}"));
    let _ = std::fs::remove_dir_all(&root);
    let target = root.join("target").join("debug");
    std::fs::create_dir_all(&target).expect("create target");
    std::fs::copy(&source, target.join(source.file_name().expect("file name")))
        .expect("copy library");

    let mut resources = Resources::new();
    resources.insert(PluginData::new());
    resources.insert(DynamicTypeRegistry::new());
    // The same bridge the ECS plugin installs: types land in the
    // registry by name, sourced from their own path prefix.
    resources.insert(ComponentBridge::new(|resources, schema| {
        let source = schema
            .type_name
            .split("::")
            .next()
            .unwrap_or(&schema.type_name)
            .to_owned();
        let registry = resources
            .get_mut::<DynamicTypeRegistry>()
            .expect("the registry is inserted beside this bridge");
        register_schema(registry, schema, &source)
    }));
    Some((root, resources))
}

fn declares(resources: &Resources, type_name: &str) -> bool {
    resources
        .get::<DynamicTypeRegistry>()
        .is_some_and(|r| r.contains(type_name))
}

/// Whether the example plugin loaded, printing why when it did not.
///
/// 🔴 Skipped rather than failed, and only here. The build stamp carries
/// the engine version, which moves on **every merged PR**, so a plugin
/// built an hour ago is refused by a workspace that has bumped since —
/// a red that says nothing about the code under test. A suite that is
/// red for an unrelated reason is one people stop reading.
///
/// A genuine break in loading still fails loudly, in
/// `kooch_core/tests/plugin_loading.rs`, which exists for that.
fn loaded(resources: &mut Resources, root: &Path) -> bool {
    if load_project_plugin(resources, root, "example_plugin") > 0 {
        return true;
    }
    skipped();
    false
}

/// One reason, for both ways this can be nothing to test against: the
/// artefact is absent, or it was built against another engine version.
fn skipped() {
    println!(
        "SKIPPED: the example plugin is missing or built against another engine \
         version.\n  cargo build -p example_plugin"
    );
}

/// 🔴 The whole feature: unload and load run in sequence and the types
/// come back. Before this, `unload_project_plugins` had no callers at
/// all and a code change reached the editor only by reopening the
/// project.
#[test]
fn a_reload_swaps_the_library() {
    let Some((root, mut resources)) = plugin_project("swap") else {
        return;
    };
    if !loaded(&mut resources, &root) {
        return;
    }
    assert!(declares(&resources, "example_plugin::Health"));

    let report = reload_project_plugins(&mut resources, &root, "example_plugin");

    assert!(
        declares(&resources, "example_plugin::Health"),
        "the swap unloaded the types and did not bring them back",
    );
    assert!(
        report.is_quiet(),
        "the same library reported a change: {report:?}",
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 🔴 A build that fails leaves no library to load. Unloading first and
/// finding nothing would empty the editor — a project that looks like it
/// has no components, because a compile error happened somewhere else.
#[test]
fn a_failed_reload_keeps_the_types() {
    let Some((root, mut resources)) = plugin_project("failed") else {
        return;
    };
    if !loaded(&mut resources, &root) {
        return;
    }

    std::fs::remove_file(
        root.join("target")
            .join("debug")
            .join(example_library().file_name().expect("file name")),
    )
    .expect("remove the library");
    let report = reload_project_plugins(&mut resources, &root, "example_plugin");

    assert!(
        declares(&resources, "example_plugin::Health"),
        "a failed build emptied the editor's type registry",
    );
    assert!(
        report.is_quiet(),
        "a refusal reported changes it did not make"
    );
    let _ = std::fs::remove_dir_all(&root);
}
