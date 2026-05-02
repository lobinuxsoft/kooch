//! ome_scripting — scripting subsystem.
//!
//! [`ScriptingBackend`] is the trait the engine consumes; concrete impls
//! ([`RhaiBackend`] today, future `MluaBackend`, WASM) plug behind it.
//!
//! # Architecture
//!
//! - [`backend`] — trait + cross-backend [`ScriptValue`] / [`ScriptError`]
//! - [`rhai_backend`] — concrete [`RhaiBackend`] (Rhai 1.21 + `sync`)
//!
//! # Out of scope (follow-ups)
//!
//! - ECS bindings auto-generated from `Reflect` (#76)
//! - Hot reload via file watcher (#75)
//! - Debug hooks / breakpoints (#77)
//! - Custom function registration through the trait (currently lives on
//!   the concrete backend via `engine_mut`)

pub mod backend;
pub mod rhai_backend;

pub use backend::{ScriptError, ScriptHandle, ScriptValue, ScriptingBackend};
pub use rhai_backend::RhaiBackend;
