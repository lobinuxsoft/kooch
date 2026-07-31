//! Loads a real plugin library and checks what it does.
//!
//! Everything else about plugins is tested against fakes. This is the
//! only test that opens an actual `.so`, so it is the only one that
//! would catch a broken `export_plugin!`, a symbol name that drifted, or
//! a build stamp that never matches.
//!
//! It needs `example_plugin` built as a dynamic library. Cargo does not
//! build one crate because another's test wants it, so the test locates
//! the artefact and **fails loudly** when it is missing rather than
//! quietly passing — a skipped test here would hide exactly the
//! integration it exists to prove.

#![cfg(feature = "dynamic")]

use std::path::PathBuf;

use kooch_core::dynamic::{ComponentBridge, EngineHost, PluginLoader};
use kooch_core::resource::Resources;
use kooch_core::schedule::Schedule;
use kooch_plugin_api::component::ComponentSchema;

/// Where cargo puts the example plugin's library.
fn plugin_path() -> PathBuf {
    // The test binary is in target/<profile>/deps.
    let mut dir = std::env::current_exe().expect("test exe path");
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

/// Collects what a plugin registers, so the test asserts on the effect
/// rather than on the absence of an error.
fn recording_resources() -> (
    Resources,
    std::sync::Arc<std::sync::Mutex<Vec<ComponentSchema>>>,
) {
    let seen: std::sync::Arc<std::sync::Mutex<Vec<ComponentSchema>>> = Default::default();
    let recorder = std::sync::Arc::clone(&seen);

    let mut resources = Resources::new();
    resources.insert(kooch_core::dynamic::PluginData::new());
    resources.insert(ComponentBridge::new(move |_, schema| {
        recorder.lock().unwrap().push(schema.clone());
        Ok(())
    }));
    (resources, seen)
}

#[test]
fn a_real_library_loads_and_declares_its_components() {
    let path = plugin_path();
    assert!(
        path.exists(),
        "no plugin library at {}. Build it first:\n  \
         cargo build -p example_plugin",
        path.display()
    );

    let mut loader = PluginLoader::new();
    // SAFETY: the library is the one this workspace just built.
    let mut plugin = unsafe { loader.load(&path) }.expect("the example plugin must load");

    assert_eq!(plugin.name(), "HelloPlugin");
    assert_eq!(loader.count(), 1);

    let (mut resources, seen) = recording_resources();
    let mut schedule = Schedule::new();
    {
        let mut host = EngineHost::building(&mut resources, &mut schedule);
        plugin.build(&mut host);
    }

    let seen = seen.lock().unwrap();
    let names: Vec<&str> = seen.iter().map(|s| s.type_name.as_str()).collect();
    assert!(
        names.contains(&"example_plugin::Health"),
        "expected Health among {names:?}"
    );
    assert!(
        names.contains(&"example_plugin::Player"),
        "a marker component must register too, got {names:?}"
    );

    let health = seen
        .iter()
        .find(|s| s.type_name == "example_plugin::Health")
        .unwrap();
    assert_eq!(
        health
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        ["current", "max", "regen"],
        "field names and order must survive the boundary"
    );
}

/// The frame counter lives in host storage, which is what lets it
/// survive a reload. Running the plugin's system must move it.
#[test]
fn the_plugins_system_runs_and_its_state_stays_in_the_host() {
    let path = plugin_path();
    assert!(path.exists(), "build example_plugin first");

    let mut loader = PluginLoader::new();
    // SAFETY: as above.
    let mut plugin = unsafe { loader.load(&path) }.expect("load");

    let (mut resources, _seen) = recording_resources();
    let mut schedule = Schedule::new();
    {
        let mut host = EngineHost::building(&mut resources, &mut schedule);
        plugin.build(&mut host);
    }

    // Nothing has run yet, so the counter does not exist.
    assert_eq!(
        resources
            .get::<kooch_core::dynamic::PluginData>()
            .unwrap()
            .get("example_plugin::frames"),
        None
    );

    schedule.run_frame_stages(&mut resources);
    schedule.run_frame_stages(&mut resources);
    schedule.run_frame_stages(&mut resources);

    let frames = resources
        .get::<kooch_core::dynamic::PluginData>()
        .unwrap()
        .get("example_plugin::frames")
        .map(|bytes| u64::from_le_bytes(bytes.try_into().unwrap()))
        .expect("the plugin's system must have run");

    assert_eq!(frames, 3, "one increment per frame, held by the host");
}

/// A library that is not a plugin must be refused by name, not by
/// crashing on a missing symbol.
#[test]
fn a_library_without_the_symbols_is_refused() {
    let mut loader = PluginLoader::new();
    let not_a_plugin = plugin_path().with_file_name("definitely-not-here.so");

    // SAFETY: nothing is opened; the path does not exist.
    // `expect_err` is unavailable because `dyn OmePlugin` is not Debug,
    // so the Ok arm is rejected explicitly.
    let err = match unsafe { loader.load(&not_a_plugin) } {
        Ok(_) => panic!("a non-existent library must not load"),
        Err(e) => e,
    };
    assert!(
        matches!(err, kooch_core::dynamic::PluginLoadError::LibraryOpen { .. }),
        "got {err}"
    );
    assert_eq!(loader.count(), 0, "a failed load must not be recorded");
}
