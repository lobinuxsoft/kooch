//! Adapts a loaded [`OmePlugin`] to the engine's own [`Plugin`] trait,
//! so a dynamic plugin goes through the same lifecycle as a static one.

use std::cell::UnsafeCell;

use kooch_plugin_api::OmePlugin;

use crate::app::App;
use crate::plugin::Plugin;

use super::host::EngineHost;
use super::plugin_data::PluginData;
use super::resource_registry::ResourceRegistry;

/// Wraps a loaded plugin as an engine [`Plugin`].
///
/// `UnsafeCell` because [`Plugin::finish`] only offers `&self`, while
/// driving the plugin's `build` needs `&mut`. `finish` is called
/// sequentially from `App::finish_plugins`, so no other reference to the
/// inner value exists while it is used.
pub struct DynamicPlugin {
    inner: UnsafeCell<Option<Box<dyn OmePlugin>>>,
}

// SAFETY: `OmePlugin` requires `Send + Sync`, and the engine drives
// plugins from a single thread.
unsafe impl Send for DynamicPlugin {}
unsafe impl Sync for DynamicPlugin {}

impl DynamicPlugin {
    /// Wraps a loaded plugin instance.
    pub fn new(plugin: Box<dyn OmePlugin>) -> Self {
        Self {
            inner: UnsafeCell::new(Some(plugin)),
        }
    }
}

impl Plugin for DynamicPlugin {
    fn build(&self, app: &mut App) {
        if !app.resources.contains::<ResourceRegistry>() {
            let mut registry = ResourceRegistry::new();
            registry.register::<crate::time::Time>("kooch_core::Time");
            app.resources.insert(registry);
        }
        if !app.resources.contains::<PluginData>() {
            app.resources.insert(PluginData::new());
        }
    }

    fn finish(&self, app: &mut App) {
        // Deliberately in `finish` rather than `build`: by now every
        // static plugin has run, so the ECS bridges and Time exist.
        //
        // SAFETY: called sequentially from `App::finish_plugins`, with
        // no other reference to `inner` alive.
        let inner = unsafe { &mut *self.inner.get() };

        if let Some(plugin) = inner {
            let mut host = EngineHost::building(&mut app.resources, &mut app.schedule);
            plugin.build(&mut host);
        }
    }

    fn name(&self) -> &str {
        "DynamicPlugin"
    }
}

impl Drop for DynamicPlugin {
    fn drop(&mut self) {
        if let Some(mut plugin) = self.inner.get_mut().take() {
            plugin.cleanup();
        }
    }
}
