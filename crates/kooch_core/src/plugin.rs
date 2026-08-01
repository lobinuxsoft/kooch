//! Plugin system for modular engine functionality.
//!
//! Plugins encapsulate related functionality (resources, systems, events)
//! that can be added to an App in a self-contained way.

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
    /// Called when the plugin is added to the app.
    ///
    /// Use this phase for:
    /// - Registering resources
    /// - Adding systems
    /// - Registering event types
    /// - Adding other plugins
    fn build(&self, app: &mut App);

    /// Called after all plugins have been built.
    ///
    /// Use this phase for:
    /// - Setting up functionality that depends on other plugins
    /// - Overriding the app runner
    /// - Final initialization
    ///
    /// Default implementation does nothing.
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
///
/// Includes only core functionality without windowing or rendering.
/// Useful for tests, servers, or CLI tools.
pub struct MinimalPlugins;

impl PluginGroup for MinimalPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::new().add(CorePlugin)
    }
}

/// Core plugin providing essential engine functionality.
///
/// Automatically included by `MinimalPlugins`. Provides:
/// - Time resource
/// - AppExit event
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
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct TestPlugin {
        built: Arc<AtomicBool>,
        finished: Arc<AtomicBool>,
    }

    impl Plugin for TestPlugin {
        fn build(&self, _app: &mut App) {
            self.built.store(true, Ordering::SeqCst);
        }

        fn finish(&self, _app: &mut App) {
            self.finished.store(true, Ordering::SeqCst);
        }

        fn name(&self) -> &str {
            "TestPlugin"
        }
    }

    #[test]
    fn plugin_build_and_finish() {
        let built = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));

        let plugin = TestPlugin {
            built: built.clone(),
            finished: finished.clone(),
        };

        let mut app = App::new();
        app.add_plugin(plugin);
        app.finish_plugins();

        assert!(built.load(Ordering::SeqCst));
        assert!(finished.load(Ordering::SeqCst));
    }

    struct PluginA;
    struct PluginB;

    impl Plugin for PluginA {
        fn build(&self, _app: &mut App) {}
        fn name(&self) -> &str {
            "PluginA"
        }
    }

    impl Plugin for PluginB {
        fn build(&self, _app: &mut App) {}
        fn name(&self) -> &str {
            "PluginB"
        }
    }

    struct TestGroup;

    impl PluginGroup for TestGroup {
        fn build(self) -> PluginGroupBuilder {
            PluginGroupBuilder::new().add(PluginA).add(PluginB)
        }
    }

    #[test]
    fn plugin_group_builder() {
        let plugins = TestGroup.build().finish();
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].name(), "PluginA");
        assert_eq!(plugins[1].name(), "PluginB");
    }

    #[test]
    fn minimal_plugins() {
        let plugins = MinimalPlugins.build().finish();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name(), "CorePlugin");
    }
}
