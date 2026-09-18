//! Plugin API — the only crate a plugin depends on. A plugin is a Rust `dylib` built by the same
//! compiler as the engine, so the API is plain Rust; the loader verifies that with a
//! [`BuildStamp`] before calling anything.
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
//! `dylib`, not `cdylib`, which cannot carry Rust types. Build both sides with `-C prefer-dynamic`,
//! or each gets its own `std` and log subscriber.
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
//! **A plugin owns no state that must survive a reload** — anything persistent belongs to the host,
//! via [`Engine::set_data`].

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
    pub use crate::types::{Order, Stage, pack_entity, unpack_entity};
    pub use crate::version::API_VERSION;
}
