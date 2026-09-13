//! `kooch_input` — [`InputBackend`] is what the engine reads (winit + gilrs in a game, a remote
//! backend behind Play, a mock in tests), with authored actions on top.
//! [`InputPlugin`] ships in `DefaultPlugins`, so a project reads the resource:
//!
//! ```ignore
//! use kooch::prelude::*;
//!
//! fn move_player(resources: &mut Resources) {
//!     let Some(input) = resources.get::<Box<dyn InputBackend>>() else { return };
//!     if input.is_pressed(KeyCode::KeyW) { /* … */ }
//! }
//! ```

pub mod actions;
pub mod backend;
pub mod ids;
pub mod mock_backend;
pub mod plugin;
pub mod remote_backend;
pub mod winit_gilrs_backend;

pub use backend::{
    GamepadAxis, GamepadButton, GamepadId, InputBackend, InputEvent, KeyCode, MouseButton,
};
pub use mock_backend::MockInputBackend;
pub use plugin::{InputPlugin, PendingWindowEvents};
pub use remote_backend::{GamepadSnapshot, InputSnapshot, RemoteInputBackend};
pub use winit_gilrs_backend::WinitGilrsBackend;
