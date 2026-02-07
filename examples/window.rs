//! Minimal example: open a window with WindowPlugin and GPU context.
//!
//! Run with: cargo run --example window

use ome_core::prelude::*;
use ome_window::{WindowCloseRequested, WindowHandle, WindowPlugin, WindowResized};

fn startup(resources: &mut Resources) {
    if let Some(handle) = resources.get::<WindowHandle>() {
        let (w, h) = handle.inner_size();
        tracing::info!("Window ready: {}x{}", w, h);
    }

    if let Some(gpu) = resources.get::<GpuContext>() {
        let info = gpu.adapter_info();
        tracing::info!(
            name = info.name,
            backend = ?info.backend,
            driver = info.driver,
            "GPU initialized"
        );
    }
}

fn on_resize(resources: &mut Resources) {
    if let Some(events) = resources.get::<Events<WindowResized>>() {
        for event in events.read() {
            tracing::info!("Resized to {}x{}", event.width, event.height);
        }
    }
}

fn on_close(resources: &mut Resources) {
    if let Some(events) = resources.get::<Events<WindowCloseRequested>>() {
        for _ in events.read() {
            tracing::info!("Close requested — goodbye!");
        }
    }
}

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin::default());
    app.add_system(Stage::Startup, startup);
    app.add_system(Stage::Input, on_resize);
    app.add_system(Stage::Input, on_close);
    app.run();
}
