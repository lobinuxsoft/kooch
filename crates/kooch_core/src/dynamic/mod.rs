//! Loading plugins from Rust dynamic libraries.
//!
//! Gated behind the `dynamic` Cargo feature.
//!
//! - [`PluginLoader`] — opens libraries and refuses incompatible ones
//! - [`EngineHost`] — the engine's implementation of the plugin API
//! - [`DynamicPlugin`] — wraps a loaded plugin as an engine [`Plugin`]
//! - [`EntityBridge`] / [`ComponentBridge`] — hooks the ECS installs
//! - [`ResourceRegistry`] — name → `TypeId` for resource lookup
//! - [`PluginData`] — host-owned storage that outlives a reload
//!
//! ```text
//! App::load_plugin(path)
//!   → PluginLoader::load()       build stamp checked BEFORE any call
//!   → DynamicPlugin::new()       wrapped as an ordinary Plugin
//!   → app.add_plugin(wrapper)    normal lifecycle; build() gets an EngineHost
//! ```
//!
//! A plugin is a `dylib`, not a `cdylib`, and both sides are built by
//! the same compiler — so the API is plain Rust rather than a C ABI.
//! What makes that sound is the build stamp the loader verifies first;
//! see [`kooch_plugin_api::version`].

pub mod bridges;
pub mod host;
pub mod loader;
pub mod plugin_data;
pub mod resource_registry;
pub mod wrapper;

pub use bridges::{ComponentBridge, EntityBridge};
pub use host::EngineHost;
pub use loader::{PluginLoadError, PluginLoader};
// Re-exported because `PluginLoadError::Incompatible` carries one, so a
// caller that wants to tell "rebuild me" apart from "this will never
// load" already depends on the type.
pub use kooch_plugin_api::version::Incompatibility;
pub use plugin_data::PluginData;
pub use resource_registry::ResourceRegistry;
pub use wrapper::DynamicPlugin;
