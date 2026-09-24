//! Application struct - the central orchestrator of the engine.

use crate::event::{AppExit, Events};
use crate::plugin::{Plugin, PluginGroup};
use crate::resource::Resources;
use crate::runner::{Runner, default_runner};
use crate::schedule::{Order, Schedule};
use crate::stage::Stage;
use crate::system::{GpuSystem, System};

#[cfg(feature = "dynamic")]
use std::path::Path;

/// The central application struct that orchestrates the engine.
///
/// Use the builder pattern to configure the app, then call `run()` to
/// start the game loop.
///
/// # Example
/// ```ignore
/// use kooch_core::prelude::*;
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

        // Present from the start so a plugin's `build` can clone it into
        // a worker thread — the remote server spawns its listener there
        // and needs a way back to a loop that may be asleep (#656).
        app.insert_resource(crate::frame_pacing::FrameWaker::default());

        app
    }

    /// Adds a plugin to the application.
    pub fn add_plugin<P: Plugin + 'static>(&mut self, plugin: P) -> &mut Self {
        // Everything the plugin schedules happens inside `build`, so bracketing it attributes every
        // system without a word at any `add_system` call site. Restored rather than reset: a plugin
        // may add another, and the inner one must not swallow the outer one's attribution.
        let outer = self.schedule.attribute_to(plugin.source());
        plugin.build(self);
        self.schedule.attribute_to(outer);
        self.pending_plugins.push(Box::new(plugin));
        self
    }

    /// Adds a group of plugins to the application.
    pub fn add_plugins<G: PluginGroup>(&mut self, group: G) -> &mut Self {
        for plugin in group.build().finish() {
            let outer = self.schedule.attribute_to(plugin.source());
            plugin.build(self);
            self.schedule.attribute_to(outer);
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

    /// Adds a closure as a CPU system at the specified stage.
    pub fn add_system<F>(&mut self, stage: Stage, system: F) -> &mut Self
    where
        F: FnMut(&mut Resources) + Send + Sync + 'static,
    {
        self.schedule.add_system(stage, system);
        self
    }

    /// Adds a closure that runs where `order` puts it inside the stage (#392).
    pub fn add_ordered<F>(&mut self, stage: Stage, order: Order, system: F) -> &mut Self
    where
        F: FnMut(&mut Resources) + Send + Sync + 'static,
    {
        self.schedule.add_ordered(stage, order, system);
        self
    }

    /// Adds a struct implementing [`System`] at the specified stage.
    pub fn add_cpu_system(&mut self, stage: Stage, system: impl System) -> &mut Self {
        self.schedule.add_cpu_system(stage, system);
        self
    }

    /// Adds a [`GpuSystem`] at the specified stage.
    pub fn add_gpu_system(&mut self, stage: Stage, system: impl GpuSystem) -> &mut Self {
        self.schedule.add_gpu_system(stage, system);
        self
    }

    /// Adds a [`System`] that runs where `order` puts it.
    pub fn add_cpu_ordered(
        &mut self,
        stage: Stage,
        order: Order,
        system: impl System,
    ) -> &mut Self {
        self.schedule.add_cpu_ordered(stage, order, system);
        self
    }

    /// Adds a [`GpuSystem`] that runs where `order` puts it.
    pub fn add_gpu_ordered(
        &mut self,
        stage: Stage,
        order: Order,
        system: impl GpuSystem,
    ) -> &mut Self {
        self.schedule.add_gpu_ordered(stage, order, system);
        self
    }

    /// Registers an event type.
    pub fn add_event<E: Send + Sync + 'static>(&mut self) -> &mut Self {
        if !self.resources.contains::<Events<E>>() {
            self.resources.insert(Events::<E>::new());
        }
        if !self.resources.contains::<crate::event::EventUpdaters>() {
            self.resources
                .insert(crate::event::EventUpdaters::default());
        }
        if let Some(updaters) = self.resources.get_mut::<crate::event::EventUpdaters>() {
            updaters.register::<E>();
        }
        self
    }

    /// Sets a custom runner function.
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
    pub fn run(mut self) {
        self.finish_plugins();
        // After every plugin, because that is when the schedule is
        // whole. A panel reads this rather than the schedule itself,
        // which lives here and not in `Resources` (#982).
        self.publish_systems();

        let runner = self.runner.take().unwrap_or(default_runner);
        runner(self);
    }

    /// Copies the schedule's description of itself into `Resources`.
    pub fn publish_systems(&mut self) -> &mut Self {
        let catalog = self.schedule.catalog();
        self.resources.insert(catalog);
        self
    }

    /// Loads a dynamic plugin from a shared library.
    #[cfg(feature = "dynamic")]
    pub unsafe fn load_plugin(
        &mut self,
        path: &Path,
    ) -> Result<&mut Self, crate::dynamic::PluginLoadError> {
        // Ensure we have a PluginLoader resource to keep libraries alive.
        if !self.resources.contains::<crate::dynamic::PluginLoader>() {
            self.resources.insert(crate::dynamic::PluginLoader::new());
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

    /// Sends an event.
    ///
    /// Panics if the event type hasn't been registered with `add_event`.
    pub fn send_event<E: Clone + Send + Sync + 'static>(&mut self, event: E) {
        self.resources
            .get_mut::<Events<E>>()
            .expect("Event type not registered")
            .send(event);
    }

    #[cfg(test)]
    /// Returns an iterator over events of type `E` from the previous frame.
    ///
    /// Returns `None` if the event type hasn't been registered.
    pub fn read_events<E: Send + Sync + 'static>(&self) -> Option<impl Iterator<Item = &E>> {
        self.resources.get::<Events<E>>().map(|e| e.read())
    }
}

#[cfg(test)]
mod tests;
