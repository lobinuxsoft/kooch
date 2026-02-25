//! Dynamic plugin loading via shared libraries (`.dll`/`.so`).
//!
//! Gated behind the `dynamic` Cargo feature. Provides:
//!
//! - [`PluginLoader`] — loads and verifies plugin libraries
//! - [`DynamicPlugin`] — wraps a dynamic plugin as a static [`Plugin`]
//! - [`BridgeContext`] / [`EntityBridge`] — FFI bridge infrastructure
//! - [`ResourceRegistry`] — name → TypeId mapping for FFI resource access
//! - [`PluginData`] — key-value byte storage for inter-plugin communication
//!
//! # Architecture
//!
//! ```text
//! App::load_plugin(path)
//!   → PluginLoader::load()          libloading + stabby ABI check
//!   → DynamicPlugin::new(plugin)    wrap as static Plugin
//!   → app.add_plugin(wrapper)       normal plugin lifecycle
//! ```

pub mod bridge;
pub mod loader;
pub mod plugin_data;
pub mod resource_registry;
pub mod wrapper;

pub use bridge::{BridgeContext, EntityBridge};
pub use loader::{PluginLoadError, PluginLoader};
pub use plugin_data::PluginData;
pub use resource_registry::ResourceRegistry;
pub use wrapper::DynamicPlugin;
