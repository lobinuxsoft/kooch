//! Host example that loads a dynamic plugin at runtime.
//!
//! Build the plugin first:
//! ```text
//! cargo build -p example_plugin
//! ```
//!
//! Then run this example:
//! ```text
//! cargo run --example dynamic_plugin --features dynamic
//! ```

use oh_my_engine::ome_core::prelude::*;
use oh_my_engine::ome_ecs::EcsPlugin;
use std::path::PathBuf;

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(EcsPlugin);
    app.set_runner(run_for_frames);

    // Find the plugin library.
    let plugin_path = find_plugin_library();

    tracing::info!(path = %plugin_path.display(), "Loading dynamic plugin");

    // SAFETY: We trust the example plugin we just built.
    unsafe {
        app.load_plugin(&plugin_path)
            .expect("failed to load plugin");
    }

    app.run();
}

/// Locates the built `example_plugin` library in the target directory.
fn find_plugin_library() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("failed to get exe path")
        .parent()
        .expect("exe has no parent")
        .to_path_buf();

    // The exe is in target/{profile}/examples, the cdylib is in target/{profile}.
    // Walk up from the examples dir to the profile dir.
    if path.ends_with("examples") {
        path.pop();
    }

    #[cfg(target_os = "windows")]
    let lib_name = "example_plugin.dll";
    #[cfg(target_os = "linux")]
    let lib_name = "libexample_plugin.so";
    #[cfg(target_os = "macos")]
    let lib_name = "libexample_plugin.dylib";

    path.push(lib_name);

    if !path.exists() {
        panic!(
            "Plugin not found at {}. Build it first:\n  cargo build -p example_plugin",
            path.display()
        );
    }

    path
}

/// Custom runner that runs for 5 frames to demonstrate the plugin system.
fn run_for_frames(mut app: App) {
    app.finish_plugins();

    app.schedule.run_startup(&mut app.resources);

    for frame in 0..5 {
        tracing::info!(frame, "--- Frame start ---");
        app.schedule.run_frame_stages(&mut app.resources);
    }

    tracing::info!("Done — 5 frames executed successfully");
}
