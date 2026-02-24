//! Application struct - the central orchestrator of the engine.
//!
//! The `App` struct provides a builder pattern for configuring plugins,
//! resources, systems, and events, then running the game loop.

use crate::event::{AppExit, Events};
use crate::plugin::{Plugin, PluginGroup};
use crate::resource::Resources;
use crate::runner::{default_runner, Runner};
use crate::schedule::Schedule;
use crate::stage::Stage;

#[cfg(feature = "dynamic")]
use std::path::Path;

/// The central application struct that orchestrates the engine.
///
/// Use the builder pattern to configure the app, then call `run()` to
/// start the game loop.
///
/// # Example
/// ```ignore
/// use ome_core::prelude::*;
///
/// fn startup(resources: &mut Resources) {
///     tracing::info!("Game started!");
/// }
///
/// fn update(resources: &mut Resources) {
///     // Game logic
/// }
///
/// App::new()
///     .add_plugins(MinimalPlugins)
///     .add_system(Stage::Startup, startup)
///     .add_system(Stage::Update, update)
///     .run();
/// ```
pub struct App {
    /// Resource storage.
    pub resources: Resources,
    /// System schedule.
    pub schedule: Schedule,
    /// Pending plugins to be finished.
    pending_plugins: Vec<Box<dyn Plugin>>,
    /// Custom runner function.
    runner: Option<Runner>,
    /// Whether plugins have been finished.
    plugins_finished: bool,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Creates a new empty application.
    ///
    /// Consider using `add_plugins(MinimalPlugins)` to get basic functionality.
    pub fn new() -> Self {
        let mut app = Self {
            resources: Resources::new(),
            schedule: Schedule::new(),
            pending_plugins: Vec::new(),
            runner: None,
            plugins_finished: false,
        };

        // Register AppExit event by default
        app.add_event::<AppExit>();

        app
    }

    /// Adds a plugin to the application.
    ///
    /// The plugin's `build` method is called immediately.
    /// The `finish` method is called when `run()` is invoked.
    pub fn add_plugin<P: Plugin + 'static>(&mut self, plugin: P) -> &mut Self {
        plugin.build(self);
        self.pending_plugins.push(Box::new(plugin));
        self
    }

    /// Adds a group of plugins to the application.
    pub fn add_plugins<G: PluginGroup>(&mut self, group: G) -> &mut Self {
        for plugin in group.build().finish() {
            plugin.build(self);
            self.pending_plugins.push(plugin);
        }
        self
    }

    /// Inserts a resource into the application.
    ///
    /// Resources are globally accessible state.
    pub fn insert_resource<T: Send + Sync + 'static>(&mut self, resource: T) -> &mut Self {
        self.resources.insert(resource);
        self
    }

    /// Adds a system to run at the specified stage.
    pub fn add_system<F>(&mut self, stage: Stage, system: F) -> &mut Self
    where
        F: FnMut(&mut Resources) + Send + Sync + 'static,
    {
        self.schedule.add_system(stage, system);
        self
    }

    /// Registers an event type.
    ///
    /// Events must be registered before they can be sent or read.
    pub fn add_event<E: Send + Sync + 'static>(&mut self) -> &mut Self {
        self.resources.insert(Events::<E>::new());
        self
    }

    /// Sets a custom runner function.
    ///
    /// The runner takes ownership of the app and controls the game loop.
    /// Use this to integrate with window event loops (e.g., winit).
    pub fn set_runner(&mut self, runner: Runner) -> &mut Self {
        self.runner = Some(runner);
        self
    }

    /// Finishes all pending plugins by calling their `finish` methods.
    ///
    /// Called automatically by `run()`, but can be called manually for testing.
    pub fn finish_plugins(&mut self) {
        if self.plugins_finished {
            return;
        }

        let plugins = std::mem::take(&mut self.pending_plugins);
        for plugin in plugins.iter() {
            plugin.finish(self);
        }

        self.plugins_finished = true;
    }

    /// Runs the application.
    ///
    /// This finishes all plugins and starts the game loop.
    /// The method does not return until the application exits.
    pub fn run(mut self) {
        self.finish_plugins();

        let runner = self.runner.take().unwrap_or(default_runner);
        runner(self);
    }

    /// Loads a dynamic plugin from a shared library.
    ///
    /// The plugin is verified for ABI compatibility, instantiated, and
    /// registered like any static plugin. The library is kept alive for
    /// the lifetime of the application.
    ///
    /// # Safety
    ///
    /// Loading a shared library executes arbitrary code. Only load plugins
    /// you trust.
    ///
    /// # Errors
    ///
    /// Returns [`PluginLoadError`](crate::dynamic::PluginLoadError) if the
    /// library can't be loaded or the plugin is incompatible.
    #[cfg(feature = "dynamic")]
    pub unsafe fn load_plugin(
        &mut self,
        path: &Path,
    ) -> Result<&mut Self, crate::dynamic::PluginLoadError> {
        // Ensure we have a PluginLoader resource to keep libraries alive.
        if !self.resources.contains::<crate::dynamic::PluginLoader>() {
            self.resources
                .insert(crate::dynamic::PluginLoader::new());
        }

        // Remove the loader, load the plugin, re-insert.
        let mut loader = self
            .resources
            .remove::<crate::dynamic::PluginLoader>()
            .unwrap();

        let plugin = unsafe { loader.load(path)? };

        self.resources.insert(loader);

        let wrapper = crate::dynamic::DynamicPlugin::new(plugin);
        self.add_plugin(wrapper);

        Ok(self)
    }

    /// Returns a reference to the resources.
    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    /// Returns a mutable reference to the resources.
    pub fn resources_mut(&mut self) -> &mut Resources {
        &mut self.resources
    }

    /// Returns a reference to the schedule.
    pub fn schedule(&self) -> &Schedule {
        &self.schedule
    }

    /// Returns a mutable reference to the schedule.
    pub fn schedule_mut(&mut self) -> &mut Schedule {
        &mut self.schedule
    }

    /// Sends an event.
    ///
    /// Panics if the event type hasn't been registered with `add_event`.
    pub fn send_event<E: Send + Sync + 'static>(&mut self, event: E) {
        self.resources
            .get_mut::<Events<E>>()
            .expect("Event type not registered")
            .send(event);
    }

    /// Returns an iterator over events of type `E` from the previous frame.
    ///
    /// Returns `None` if the event type hasn't been registered.
    pub fn read_events<E: Send + Sync + 'static>(&self) -> Option<impl Iterator<Item = &E>> {
        self.resources.get::<Events<E>>().map(|e| e.read())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[test]
    fn new_app() {
        let app = App::new();
        assert!(app.resources.contains::<Events<AppExit>>());
    }

    #[test]
    fn insert_resource() {
        let mut app = App::new();
        app.insert_resource(42_i32);

        assert_eq!(app.resources().get::<i32>(), Some(&42));
    }

    #[test]
    fn add_system() {
        let mut app = App::new();
        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();

        app.add_system(Stage::Update, move |_| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        app.schedule.run_stage(Stage::Update, &mut app.resources);

        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[derive(Debug, Clone, PartialEq)]
    struct TestEvent(i32);

    #[test]
    fn events() {
        let mut app = App::new();
        app.add_event::<TestEvent>();

        app.send_event(TestEvent(42));

        // Not readable yet (in write buffer)
        let events: Vec<_> = app.read_events::<TestEvent>().unwrap().collect();
        assert!(events.is_empty());

        // Swap buffers
        app.resources
            .get_mut::<Events<TestEvent>>()
            .unwrap()
            .update();

        // Now readable
        let events: Vec<_> = app.read_events::<TestEvent>().unwrap().collect();
        assert_eq!(events, vec![&TestEvent(42)]);
    }

    struct TestPlugin;

    impl Plugin for TestPlugin {
        fn build(&self, app: &mut App) {
            app.insert_resource(String::from("built"));
        }

        fn finish(&self, app: &mut App) {
            if let Some(s) = app.resources_mut().get_mut::<String>() {
                s.push_str(" and finished");
            }
        }
    }

    #[test]
    fn plugin_lifecycle() {
        let mut app = App::new();
        app.add_plugin(TestPlugin);

        assert_eq!(
            app.resources().get::<String>(),
            Some(&String::from("built"))
        );

        app.finish_plugins();

        assert_eq!(
            app.resources().get::<String>(),
            Some(&String::from("built and finished"))
        );
    }
}
