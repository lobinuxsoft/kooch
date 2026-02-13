//! [`RenderPlugin`] — registers the clear-color render system.

use ome_core::app::App;
use ome_core::plugin::Plugin;
use ome_core::stage::Stage;

use crate::clear_color::ClearColor;
use crate::fps::FpsTracker;
use crate::systems::render_system;

/// Plugin that clears the screen with a solid color each frame.
///
/// # Example
/// ```ignore
/// use ome_render::RenderPlugin;
///
/// app.add_plugin(RenderPlugin::default());
/// ```
#[derive(Default)]
pub struct RenderPlugin {
    /// The background color used to clear the screen.
    pub clear_color: ClearColor,
}

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.clear_color);
        app.insert_resource(FpsTracker::new());
        app.add_system(Stage::Render, render_system);
    }

    fn name(&self) -> &str {
        "RenderPlugin"
    }
}
