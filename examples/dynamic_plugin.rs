//! Loads a plugin from a Rust dynamic library and runs a few frames.
//!
//! Both halves must be built with the same flags, or the plugin links
//! its own copy of `std` and the engine's globals fork:
//!
//! ```text
//! RUSTFLAGS="-C prefer-dynamic" cargo build -p example_plugin
//! RUSTFLAGS="-C prefer-dynamic" cargo run --example dynamic_plugin --features dynamic
//! ```
//!
//! The plugin declares two component types the engine has no Rust type
//! for, and a system that keeps a frame counter in host-owned storage —
//! the shape that survives a reload.

use oh_my_engine::ome_core::prelude::*;
use oh_my_engine::ome_ecs::EcsPlugin;
use std::path::PathBuf;

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(EcsPlugin);
    app.set_runner(run_for_frames);

    let plugin_path = find_plugin_library();
    tracing::info!(path = %plugin_path.display(), "loading plugin");

    // SAFETY: this is the library we just built, in our own target dir.
    unsafe {
        app.load_plugin(&plugin_path)
            .expect("failed to load plugin");
    }

    app.run();
}

/// Finds the built `example_plugin` library next to this example.
fn find_plugin_library() -> PathBuf {
    let mut path = std::env::current_exe()
        .expect("failed to get exe path")
        .parent()
        .expect("exe has no parent")
        .to_path_buf();

    // The example binary lands in target/{profile}/examples; the library
    // is one level up in target/{profile}.
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

    assert!(
        path.exists(),
        "plugin not found at {}. Build it first:\n  \
         RUSTFLAGS=\"-C prefer-dynamic\" cargo build -p example_plugin",
        path.display()
    );

    path
}

/// Runs enough frames for the plugin's every-60th-frame log to fire.
fn run_for_frames(mut app: App) {
    app.finish_plugins();
    app.schedule.run_startup(&mut app.resources);

    for _ in 0..120 {
        app.schedule.run_frame_stages(&mut app.resources);
    }

    tracing::info!("done — 120 frames executed");
}
