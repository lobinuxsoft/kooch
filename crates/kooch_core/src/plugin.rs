//! Plugin system for modular engine functionality.

use crate::app::App;

/// A modular unit of engine functionality.
///
/// Plugins can register resources, systems, events, and other plugins
/// during two phases: `build` (early setup) and `finish` (late setup).
///
/// # Example
/// ```ignore
/// struct GamePlugin;
///
/// impl Plugin for GamePlugin {
///     fn build(&self, app: &mut App) {
///         app.insert_resource(GameState::default())
///            .add_system(Stage::Update, game_update);
///     }
/// }
///
/// App::new().add_plugin(GamePlugin).run();
/// ```
pub trait Plugin: Send + Sync {
    /// Which half of the build this plugin's systems belong to.
    fn source(&self) -> crate::schedule::SystemSource {
        crate::schedule::SystemSource::Engine
    }

    /// Called when the plugin is added to the app.
    fn build(&self, app: &mut App);

    /// Called after all plugins have been built.
    fn finish(&self, _app: &mut App) {}

    /// Returns the plugin's name for debugging.
    ///
    /// Default implementation uses the type name.
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }
}

/// A collection of plugins that can be added together.
///
/// Useful for grouping related plugins or creating preset configurations.
///
/// # Example
/// ```ignore
/// struct DefaultPlugins;
///
/// impl PluginGroup for DefaultPlugins {
///     fn build(self) -> PluginGroupBuilder {
///         PluginGroupBuilder::new()
///             .add(CorePlugin)
///             .add(WindowPlugin)
///             .add(RenderPlugin)
///     }
/// }
///
/// App::new().add_plugins(DefaultPlugins).run();
/// ```
pub trait PluginGroup {
    /// Builds the list of plugins in this group.
    fn build(self) -> PluginGroupBuilder;
}

/// Builder for constructing a group of plugins.
pub struct PluginGroupBuilder {
    plugins: Vec<Box<dyn Plugin>>,
}

impl Default for PluginGroupBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginGroupBuilder {
    /// Creates a new empty plugin group builder.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Adds a plugin to the group.
    pub fn add<P: Plugin + 'static>(mut self, plugin: P) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    /// Returns the plugins in this group.
    pub fn finish(self) -> Vec<Box<dyn Plugin>> {
        self.plugins
    }
}

/// Minimal plugins for a headless application.
pub struct MinimalPlugins;

impl PluginGroup for MinimalPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::new().add(CorePlugin)
    }
}

/// Core plugin providing essential engine functionality.
pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut App) {
        // Before anything else has a chance to log. Without a subscriber
        // every `tracing` call in the engine is silently discarded, which
        // is a failure mode that looks exactly like nothing happening.
        crate::init_tracing_if_needed();

        use crate::coord::ActiveOrigin;
        use crate::event::AppExit;
        use crate::frame_metrics::{FrameMetrics, MetricsReport, frame_metrics_system};
        use crate::time::Time;

        app.insert_resource(Time::new());
        app.insert_resource(ActiveOrigin::default());
        app.add_event::<AppExit>();

        // Measured always, reported only if asked. Two subtractions per
        // frame is not a cost worth a switch, and a game that has to be
        // rebuilt to answer "how fast am I going" answers it too late.
        app.insert_resource(FrameMetrics::new(MetricsReport::from_env()));
        app.add_system(crate::stage::Stage::Last, frame_metrics_system);
    }

    fn name(&self) -> &str {
        "CorePlugin"
    }
}

#[cfg(test)]
mod tests;
