//! Wraps a dynamic [`OmePlugin`] as a static [`Plugin`] for the engine.
//!
//! This lets dynamic plugins participate in the normal plugin lifecycle
//! (`build` / `finish`) alongside static plugins.

use std::cell::UnsafeCell;

use ome_plugin_api::plugin::BoxedPlugin;
// Import stabby-generated traits that provide methods on Dyn trait objects.
use ome_plugin_api::plugin::OmePluginDynMut;

use crate::app::App;
use crate::plugin::Plugin;

use super::bridge::{BridgeContext, create_engine_api};
use super::plugin_data::PluginData;
use super::resource_registry::ResourceRegistry;

/// Thin wrapper that adapts a dynamic [`OmePlugin`] to the engine's [`Plugin`] trait.
///
/// Uses `UnsafeCell` because the static `Plugin` trait only gives `&self` in
/// `finish()`, but we need `&mut` to call the stabby plugin's `build()`.
pub struct DynamicPlugin {
    inner: UnsafeCell<Option<BoxedPlugin>>,
}

// SAFETY: OmePlugin requires Send + Sync on implementations. The vtable doesn't
// carry Send/Sync markers (VtDrop only), but the trait bound guarantees
// implementations are Send + Sync. The engine is also single-threaded.
unsafe impl Send for DynamicPlugin {}
unsafe impl Sync for DynamicPlugin {}

impl DynamicPlugin {
    /// Wraps a loaded plugin instance.
    pub fn new(plugin: BoxedPlugin) -> Self {
        Self {
            inner: UnsafeCell::new(Some(plugin)),
        }
    }
}

impl Plugin for DynamicPlugin {
    fn build(&self, app: &mut App) {
        // Ensure infrastructure resources exist.
        if !app.resources.contains::<ResourceRegistry>() {
            let mut registry = ResourceRegistry::new();
            // Register well-known engine resources.
            registry.register::<crate::time::Time>("ome_core::Time");
            app.resources.insert(registry);
        }
        if !app.resources.contains::<PluginData>() {
            app.resources.insert(PluginData::new());
        }
    }

    fn finish(&self, app: &mut App) {
        // We call the dynamic plugin's build() in finish() because at this
        // point all static plugins have already run build(), so resources
        // like EntityAllocator, Time, etc. are available.
        //
        // SAFETY: finish() is called sequentially from App::finish_plugins.
        // No other references to inner exist at this point.
        let inner = unsafe { &mut *self.inner.get() };

        if let Some(plugin) = inner {
            let mut bridge_ctx = BridgeContext {
                resources: &mut app.resources as *mut crate::resource::Resources,
                schedule: &mut app.schedule as *mut crate::schedule::Schedule,
            };
            let mut api = create_engine_api(&mut bridge_ctx);
            plugin.build(&mut api as *mut ome_plugin_api::EngineApi);
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
