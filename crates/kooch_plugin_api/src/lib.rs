//! Plugin API for Kooch — the only crate a plugin depends on.
//!
//! A plugin is a Rust `dylib` the engine loads at run time. It defines:
//!
//! - [`KoochPlugin`] — the trait a plugin implements
//! - [`Engine`] — what a plugin can ask the engine to do
//! - [`component`] — describing component types the engine cannot name
//! - [`types`] — stages and entity handles
//! - [`version`] — the build stamp that makes the whole thing sound
//!
//! # Why plain Rust types
//!
//! Plugin and engine are built by the same compiler, in the same
//! workspace, moments apart. Under that condition Rust types are
//! compatible across the boundary, so the API is ordinary Rust: traits,
//! `String`, `Vec`, `Box<dyn Fn>`. No function-pointer tables, no
//! `*mut c_void`, no stable-ABI dependency.
//!
//! **That condition is the entire contract, so it is verified rather
//! than assumed.** Every plugin exports a [`BuildStamp`](version::BuildStamp)
//! recording its API version and the exact compiler that produced it,
//! and the loader compares it before calling anything else. A mismatch
//! is a refusal with a message naming which half is wrong.
//!
//! This is deliberately not a stable ABI. A plugin is not something a
//! third party compiles once and ships against future engine versions —
//! it is your project's code, rebuilt alongside the engine. Buying ABI
//! stability would cost a dependency and a layer of indirection to solve
//! a problem this design does not have.
//!
//! # Project setup
//!
//! ```toml
//! [lib]
//! crate-type = ["rlib", "dylib"]
//!
//! [dependencies]
//! kooch_plugin_api = { path = "..." }
//! ```
//!
//! `dylib`, not `cdylib`: a `cdylib` exposes a C interface and cannot
//! carry Rust types. The `rlib` alongside it keeps ordinary consumers
//! building.
//!
//! Both sides must be built with `-C prefer-dynamic`, or each ends up
//! with its own copy of `std` and of the engine's globals — including
//! the log subscriber, which would silently swallow the plugin's output.
//!
//! # Writing one
//!
//! ```ignore
//! use kooch_plugin_api::prelude::*;
//!
//! #[derive(Default)]
//! struct MyPlugin;
//!
//! impl KoochPlugin for MyPlugin {
//!     fn name(&self) -> &str { "MyPlugin" }
//!
//!     fn build(&mut self, engine: &mut dyn Engine) {
//!         engine.register_component(
//!             ComponentSchema::new("my_game::Health")
//!                 .with_field("current", FieldKind::U32),
//!         ).expect("Health");
//!     }
//! }
//!
//! kooch_plugin_api::export_plugin!(MyPlugin);
//! ```
//!
//! # The one rule
//!
//! **A plugin owns no state that must survive a reload.** The library is
//! unloaded and replaced; its statics go with it. Anything that has to
//! persist belongs to the host, via [`Engine::set_data`].

pub mod component;
pub mod engine_api;
pub mod plugin;
pub mod types;
pub mod version;

pub use component::{ComponentSchema, FieldKind, FieldSchema, RegisterError};
pub use engine_api::{Engine, PluginSystem};
pub use plugin::{CREATE_SYMBOL, CreatePluginFn, KoochPlugin, STAMP_SYMBOL};
pub use types::Stage;
pub use version::{API_VERSION, BuildStamp};

/// Everything a plugin author needs in one import.
pub mod prelude {
    pub use crate::component::{ComponentSchema, FieldKind, FieldSchema, RegisterError};
    pub use crate::engine_api::{Engine, PluginSystem};
    pub use crate::plugin::KoochPlugin;
    pub use crate::types::{Stage, pack_entity, unpack_entity};
    pub use crate::version::API_VERSION;
}
