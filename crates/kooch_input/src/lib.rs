//! kooch_input — input subsystem.
//!
//! [`InputBackend`] is the trait the engine consumes; concrete impls
//! (`WinitGilrsBackend`, `MockInputBackend`, future SDL2 / Steam Input)
//! plug behind it. [`ActionMap`] sits on top, mapping typed actions to
//! one or more input bindings so gameplay code reads `is_pressed(Jump)`
//! without caring whether `Jump` is `Space`, `GamepadButton::South`,
//! or `MouseButton::Right` today.
//!
//! # Architecture
//!
//! - [`backend`] — public trait + re-exported winit / gilrs types
//! - [`winit_gilrs_backend`] — production backend
//! - [`mock_backend`] — headless backend for tests + tooling
//! - [`action_map`] — typed action ↔ binding registry
//! - [`plugin`] — what connects all of the above to a running app
//!
//! # Reaching this from a game
//!
//! [`InputPlugin`] ships in `DefaultPlugins`, so a project reads input
//! straight off the resource:
//!
//! ```ignore
//! use kooch::prelude::*;
//!
//! fn move_player(resources: &mut Resources) {
//!     let Some(input) = resources.get::<Box<dyn InputBackend>>() else { return };
//!     if input.is_pressed(KeyCode::KeyW) { /* … */ }
//! }
//! ```

pub mod action_map;
pub mod backend;
pub mod ids;
pub mod mock_backend;
pub mod plugin;
pub mod remote_backend;
pub mod winit_gilrs_backend;

pub use action_map::{Action, ActionMap, InputBinding};
pub use backend::{
    GamepadAxis, GamepadButton, GamepadId, InputBackend, InputEvent, KeyCode, MouseButton,
};
pub use mock_backend::MockInputBackend;
pub use plugin::{InputPlugin, PendingWindowEvents};
pub use remote_backend::{GamepadSnapshot, InputSnapshot, RemoteInputBackend};
pub use winit_gilrs_backend::WinitGilrsBackend;
