//! ABI-stable plugin API for OhMyEngine.
//!
//! This crate is the **only dependency** plugin authors need. It defines:
//!
//! - [`OmePlugin`] — The trait plugins implement (stabby vtable)
//! - [`EngineApi`] — Function pointer table for engine services
//! - [`types`] — Stage constants, entity packing, callback signatures
//! - [`version`] — API version constants and compatibility check
//!
//! # Architecture
//!
//! ```text
//! ome_plugin_api (this crate)     ← plugin authors depend ONLY on this
//!     ↑
//! ome_core [feature: dynamic]     ← engine loads plugins at runtime
//!     ↑
//! oh_my_engine (facade)           ← re-exports everything
//! ```
//!
//! # Plugin Author Guide
//!
//! ## Project Setup
//!
//! Create a new crate with `crate-type = ["cdylib"]`:
//!
//! ```toml
//! [package]
//! name = "my_plugin"
//! version = "0.1.0"
//! edition = "2024"
//!
//! [lib]
//! crate-type = ["cdylib"]
//!
//! [dependencies]
//! ome_plugin_api = { version = "0.1" }
//! stabby = "72"
//! ```
//!
//! ## Implementing a Plugin
//!
//! Every plugin must:
//! 1. Implement the [`OmePlugin`] trait
//! 2. Export a constructor function named `ome_create_plugin` via `#[stabby::export]`
//! 3. Return [`BoxedPlugin`] from the constructor
//!
//! ```ignore
//! use ome_plugin_api::prelude::*;
//! use ome_plugin_api::BoxedPlugin;
//!
//! struct MyPlugin;
//!
//! impl OmePlugin for MyPlugin {
//!     extern "C" fn name(&self) -> stabby::string::String {
//!         "MyPlugin".into()
//!     }
//!
//!     extern "C" fn api_version(&self) -> u32 {
//!         API_VERSION
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
//! extern "C" fn ome_create_plugin() -> BoxedPlugin {
//!     stabby::alloc::boxed::Box::new(MyPlugin).into()
//! }
//! ```
//!
//! ## Lifecycle
//!
//! 1. Engine loads the `.dll`/`.so` and calls `ome_create_plugin()`
//! 2. Engine checks [`OmePlugin::api_version`] — rejects incompatible plugins
//! 3. Engine calls [`OmePlugin::build`] — register systems and access resources here
//! 4. Game loop runs — registered system callbacks execute each frame
//! 5. Engine calls [`OmePlugin::cleanup`] on shutdown
//!
//! ## Registering Systems
//!
//! Use [`EngineApi::register_system`] during `build()` to add per-frame logic:
//!
//! ```ignore
//! // System without state (null userdata).
//! api.register_system(stage::UPDATE, my_system, std::ptr::null_mut(), None);
//!
//! // System with state (heap-allocated userdata + destructor).
//! let state = Box::into_raw(Box::new(MyState::default())) as *mut c_void;
//! api.register_system(stage::UPDATE, stateful_system, state, Some(drop_state));
//! ```
//!
//! Each system callback receives a fresh [`EngineApi`] pointer valid for that
//! invocation. See [`types::stage`] for available stages.
//!
//! ## Accessing Engine Resources
//!
//! Plugins can read/write engine resources by name:
//!
//! ```ignore
//! extern "C" fn my_system(api: *mut EngineApi, _: *mut c_void) {
//!     let api = unsafe { &mut *api };
//!     let ptr = api.resource_ptr("ome_core::Time");
//!     if !ptr.is_null() {
//!         // Cast to the concrete type (must match exactly).
//!         let time = unsafe { &*(ptr as *const ome_core::time::Time) };
//!     }
//! }
//! ```
//!
//! Only resources registered in the engine's `ResourceRegistry` are accessible.
//!
//! ## Plugin-to-Plugin Communication
//!
//! [`EngineApi::set_data`] and [`EngineApi::get_data`] provide a key-value byte
//! store shared across all plugins. Use this for cross-plugin state:
//!
//! ```ignore
//! // Writer plugin
//! api.set_data("score.current", &score.to_le_bytes());
//!
//! // Reader plugin
//! if let Some(bytes) = api.get_data("score.current") {
//!     let score = u32::from_le_bytes(bytes.try_into().unwrap());
//! }
//! ```
//!
//! ## Entity Management
//!
//! Spawn and despawn entities via packed `u64` handles:
//!
//! ```ignore
//! let entity = api.spawn();
//! // ... later ...
//! api.despawn(entity);
//! ```
//!
//! Use [`types::unpack_entity`] to extract `(index, generation)` if needed.
//!
//! ## Building and Loading
//!
//! ```bash
//! # Build the plugin
//! cargo build -p my_plugin --release
//!
//! # The engine loads it at runtime
//! # (produces my_plugin.dll on Windows, libmy_plugin.so on Linux)
//! ```
//!
//! On the engine side:
//!
//! ```ignore
//! unsafe { app.load_plugin(Path::new("plugins/my_plugin.dll"))? };
//! ```
//!
//! ## Safety Notes
//!
//! - All `extern "C" fn` methods must not panic across the FFI boundary
//! - The `api` pointer in `build()` and system callbacks is only valid for
//!   the duration of that call — do not store it
//! - Resource pointers are invalidated after the callback returns
//! - Userdata must be `Send + Sync` safe (the engine is single-threaded
//!   but the type system requires these bounds)

pub mod engine_api;
pub mod plugin;
pub mod types;
pub mod version;

pub use engine_api::EngineApi;
pub use plugin::{BoxedPlugin, OmePlugin};

/// Re-exports for plugin authors.
pub mod prelude {
    pub use crate::engine_api::EngineApi;
    pub use crate::plugin::OmePlugin;
    pub use crate::types::{stage, pack_entity, unpack_entity, SystemCallback, UserdataDrop};
    pub use crate::version::API_VERSION;
}
