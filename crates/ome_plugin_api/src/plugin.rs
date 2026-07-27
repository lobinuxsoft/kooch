//! ABI-stable plugin trait.
//!
//! Plugins implement [`OmePlugin`] and export a constructor via `#[stabby::export]`.
//! The engine loads the constructor at runtime and drives the plugin lifecycle.
//!
//! # Example (plugin side)
//!
//! ```ignore
//! use ome_plugin_api::prelude::*;
//!
//! struct MyPlugin;
//!
//! impl OmePlugin for MyPlugin {
//!     extern "C" fn name(&self) -> stabby::string::String {
//!         "MyPlugin".into()
//!     }
//!
//!     extern "C" fn api_version(&self) -> u32 {
//!         ome_plugin_api::version::API_VERSION
//!     }
//!
//!     extern "C" fn build(&mut self, api: *mut EngineApi) {
//!         let api = unsafe { &mut *api };
//!         api.log("MyPlugin loaded!");
//!     }
//!
//!     extern "C" fn cleanup(&mut self) {}
//! }
//!
//! #[stabby::export]
//! extern "C" fn ome_create_plugin() -> stabby::dynptr!(Box<dyn OmePlugin>) {
//!     stabby::alloc::boxed::Box::new(MyPlugin).into()
//! }
//! ```

use crate::engine_api::EngineApi;

/// ABI-stable plugin interface with stabby vtable.
///
/// # Lifecycle
///
/// 1. Engine calls [`api_version()`](OmePlugin::api_version) — rejects incompatible plugins
/// 2. Engine calls [`name()`](OmePlugin::name) — used for logging
/// 3. Engine calls [`build(api)`](OmePlugin::build) — plugin registers systems/resources
/// 4. Game loop runs (registered systems execute each frame)
/// 5. Engine calls [`cleanup()`](OmePlugin::cleanup) — plugin releases resources
#[stabby::stabby]
pub trait OmePlugin: Send + Sync {
    /// Returns the plugin name (for logging and diagnostics).
    extern "C" fn name(&self) -> stabby::string::String;

    /// Returns the API version this plugin was built against.
    ///
    /// Should return [`version::API_VERSION`](crate::version::API_VERSION).
    extern "C" fn api_version(&self) -> u32;

    /// Called once during plugin loading.
    ///
    /// Register systems, access resources, and perform initialization here.
    /// The `api` pointer is valid only for the duration of this call.
    extern "C" fn build(&mut self, api: *mut EngineApi);

    /// Called when the engine shuts down. Release plugin resources here.
    extern "C" fn cleanup(&mut self);
}

/// ABI-stable boxed plugin trait object.
///
/// Manually expanded from `stabby::dynptr!(Box<dyn OmePlugin>)` because the
/// `dynptr!` macro resolves `Box` to `std::boxed::Box` in Rust edition 2024,
/// which doesn't implement `IPtrOwned`. We use `stabby::alloc::boxed::Box`
/// explicitly.
pub type BoxedPlugin = stabby::abi::Dyn<
    'static,
    stabby::alloc::boxed::Box<()>,
    <dyn OmePlugin as stabby::abi::vtable::CompoundVt<'static>>::Vt<stabby::abi::vtable::VtDrop>,
>;

/// Constructor function signature exported by plugin cdylibs.
///
/// Plugins export this as `ome_create_plugin` via `#[stabby::export]`.
pub type CreatePluginFn = extern "C" fn() -> BoxedPlugin;
