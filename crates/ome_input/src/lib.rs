//! ome_input — input subsystem.
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

pub mod action_map;
pub mod backend;
pub mod mock_backend;
pub mod winit_gilrs_backend;

pub use action_map::{Action, ActionMap, InputBinding};
pub use backend::{
    GamepadAxis, GamepadButton, GamepadId, InputBackend, InputEvent, KeyCode, MouseButton,
};
pub use mock_backend::MockInputBackend;
pub use winit_gilrs_backend::WinitGilrsBackend;
