//! Minimal example: clear the screen with a solid color.
//!
//! Run with: cargo run --example clear_color

use ome_core::prelude::*;
use ome_render::{ClearColor, RenderPlugin};
use ome_window::WindowPlugin;

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin::default());
    app.add_plugin(RenderPlugin {
        clear_color: ClearColor {
            r: 0.2,
            g: 0.05,
            b: 0.3,
            a: 1.0,
        },
        ..Default::default()
    });
    app.run();
}
